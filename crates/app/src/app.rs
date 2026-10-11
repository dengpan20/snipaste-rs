//! 应用主体：winit 事件循环 + 状态机。
//!
//! ## 线程模型
//!
//! ```text
//! 主线程（winit 事件循环）      唯一能做窗口 / GPU / 菜单 操作的地方
//!   ├─ 托盘菜单事件  ← 经 EventLoopProxy 转成 UserEvent 回投
//!   ├─ 全局热键事件  ← 同上
//!   └─ 渲染           ← RedrawRequested 时同步完成
//! ```
//!
//! 跨线程只传**轻量的值**（热键 id、菜单 id），绝不传窗口或 GPU 对象。
//! 识别线程只回投结果数据，贴图创建也由主线程接收像素后完成。

use std::sync::Arc;

use anyhow::{Context as _, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId, WindowLevel};

use global_hotkey::GlobalHotKeyEvent;
use snipaste_core::style::Tool;
use snipaste_core::{Pos, Rect};
use snipaste_render::{fonts, theme, Viewport};

use crate::editor::{Editor, QrResult};
use crate::gfx::{GpuContext, SurfaceWindow};
use crate::hotkey::{Action, Hotkeys};
use crate::pin::{PinCommand, PinWindow};
use crate::tray::{MenuAction, Tray};
use crate::ui::{self, Command};
use crate::{color_picker, export, qr};

/// 从别的线程投回事件循环的用户事件。
///
/// 跨线程只传数据；窗口、GPU 和剪贴板连接留在主线程。
#[derive(Debug)]
pub enum UserEvent {
    /// 全局热键被按下（携带 `global-hotkey` 的按键 id）。
    Hotkey(u32),
    /// 托盘菜单被点击。
    Menu(MenuAction),
    QrResult {
        request: u64,
        result: Result<Vec<String>, String>,
    },
    PinImage {
        image: image::RgbaImage,
        origin: Pos,
    },
}

/// 应用。
pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    egui_ctx: egui::Context,
    editor: Editor,
    snapshot: Option<snipaste_capture::Snapshot>,
    capture_texture: Option<egui::TextureHandle>,
    clipboard: Option<snipaste_platform::clipboard::Clipboard>,
    pins: Vec<PinWindow>,
    qr_request: u64,
    modifiers: winit::keyboard::ModifiersState,

    /// 截图/贴图时按需建立，空闲三秒后释放。
    gfx: Option<GpuContext>,
    /// 遮罩窗口。多显示器版本里这里会变成一个 `Vec`。
    overlay: Option<SurfaceWindow>,
    screen_bounds: Option<Rect>,
    started: bool,
    graphics_release_deadline: Option<std::time::Instant>,

    tray: Option<Tray>,
    hotkeys: Option<Hotkeys>,

    /// 实际生效的截图热键名。F1 常常被占（本机就装了正版 Snipaste），
    /// 所以状态栏必须显示真实生效的那个键，否则用户会以为程序坏了。
    hotkey_label: String,
    /// 遮罩是否可见。
    visible: bool,

    /// 冒烟测试的退出时刻。
    ///
    /// 设了环境变量 `SNIPASTE_SMOKE_MS=3000` 时，程序会在 3 秒后自动退出并
    /// 打印已呈现的帧数。这是唯一能在无人值守环境下验证"窗口 + wgpu + 托盘 +
    /// 全局热键"整条链路真的能起来的办法（CI 上也用同一个开关）。
    smoke_deadline: Option<std::time::Instant>,
    /// 已成功 present 的帧数。
    frames_rendered: u64,
    repaint_deadline: Option<std::time::Instant>,
    memory_profile: Option<crate::memory_profile::Profile>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let egui_ctx = egui::Context::default();

        let mut app = Self {
            proxy: proxy.clone(),
            egui_ctx,
            editor: Editor::new(),
            snapshot: None,
            capture_texture: None,
            clipboard: None,
            pins: Vec::new(),
            qr_request: 0,
            modifiers: winit::keyboard::ModifiersState::empty(),
            gfx: None,
            overlay: None,
            screen_bounds: None,
            started: false,
            graphics_release_deadline: None,
            tray: None,
            hotkeys: None,
            hotkey_label: "读取中…".to_owned(),
            // 启动时**不**显示遮罩：本程序是常驻托盘的，
            // 只有热键或托盘菜单才会真正进入截图。见 `start_capture`。
            visible: false,
            smoke_deadline: None,
            frames_rendered: 0,
            repaint_deadline: None,
            memory_profile: None,
        };
        app.install_event_bridges(proxy);
        app
    }

    /// 把全局热键与托盘菜单的事件桥接成 `UserEvent`。
    ///
    /// 这两套 API 的事件回调都是**全局静态**且只能设置一次，因此只在
    /// 构造时调用一遍。回调在别的线程上执行，所以里面只做 `send_event`。
    fn install_event_bridges(&mut self, proxy: EventLoopProxy<UserEvent>) {
        let hotkey_proxy = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == global_hotkey::HotKeyState::Pressed {
                let _ = hotkey_proxy.send_event(UserEvent::Hotkey(event.id()));
            }
        }));

        let menu_proxy = proxy.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(
            move |event: tray_icon::menu::MenuEvent| {
                if let Some(action) = MenuAction::parse(&event.id) {
                    let _ = menu_proxy.send_event(UserEvent::Menu(action));
                }
            },
        ));

        // 托盘图标的点击事件：左键单击直接开截图，和 Snipaste 一致。
        tray_icon::TrayIconEvent::set_event_handler(Some(
            move |event: tray_icon::TrayIconEvent| {
                // macOS left click opens the native menu-bar menu.
                if matches!(
                    event,
                    tray_icon::TrayIconEvent::Click {
                        button: tray_icon::MouseButton::Left,
                        button_state: tray_icon::MouseButtonState::Up,
                        ..
                    }
                ) && !cfg!(target_os = "macos")
                {
                    let _ = proxy.send_event(UserEvent::Menu(MenuAction::Capture));
                }
            },
        ));
    }

    // ------------------------------------------------------------ 生命周期

    /// `resumed` 会被调用多次（Windows 上窗口状态变化时）；只在第一次建资源。
    fn ensure_started(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.started {
            return Ok(());
        }

        // ---- 托盘与热键：必须在事件循环跑起来之后 ----
        match Tray::new(snipaste_platform::autostart::enabled()) {
            Ok(t) => self.tray = Some(t),
            Err(err) => log::warn!("托盘不可用：{err:#}"),
        }
        match Hotkeys::register() {
            Ok(h) => {
                self.hotkey_label = h.capture_label.clone();
                if let Some(tray) = self.tray.as_ref() {
                    tray.set_hotkey_labels(&h.capture_label, &h.paste_label);
                }
                self.hotkeys = Some(h);
            }
            Err(err) => {
                log::error!("全局热键注册失败：{err:#}（仍可用托盘或界面按钮触发）");
                self.hotkey_label = "注册失败，请用托盘菜单".to_owned();
            }
        }

        self.started = true;
        self.memory_profile = crate::memory_profile::Profile::from_env();
        #[cfg(debug_assertions)]
        if let Ok(dir) = std::env::var("SNIPASTE_VERIFY_DIR") {
            #[cfg(target_os = "macos")]
            anyhow::ensure!(
                snipaste_platform::macos::screen_permission(true),
                "请允许 Snipaste-RS Verify 录制屏幕，然后重新运行验收脚本"
            );
            self.ensure_graphics(event_loop)?;
            self.verify_native(event_loop, std::path::Path::new(&dir))?;
            event_loop.exit();
            return Ok(());
        }
        if let Ok(raw) = std::env::var("SNIPASTE_SMOKE_MS") {
            match raw.parse::<u64>() {
                Ok(ms) => {
                    let ms = ms.clamp(200, 60_000);
                    self.smoke_deadline =
                        Some(std::time::Instant::now() + std::time::Duration::from_millis(ms));
                    log::info!("冒烟模式已启用：{ms} ms 后自动退出");
                    self.start_capture(event_loop);
                }
                Err(_) => log::warn!("SNIPASTE_SMOKE_MS 不是合法毫秒数：{raw}"),
            }
        }
        Ok(())
    }

    fn ensure_graphics(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        self.graphics_release_deadline = None;
        if self.overlay.is_some() {
            return Ok(());
        }
        // 字体仅在需要界面时加载，并在首帧布局前准备好。
        self.egui_ctx = egui::Context::default();
        theme::apply(&self.egui_ctx);
        fonts::install(&self.egui_ctx);

        // ---- 遮罩窗口 ----
        #[cfg(not(target_os = "macos"))]
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .context("找不到任何显示器")?;

        #[cfg(not(target_os = "macos"))]
        let (size, position, scale) = {
            let monitors: Vec<_> = event_loop.available_monitors().collect();
            let left = monitors.iter().map(|m| m.position().x).min().unwrap_or(0);
            let top = monitors.iter().map(|m| m.position().y).min().unwrap_or(0);
            let right = monitors
                .iter()
                .map(|m| m.position().x + m.size().width as i32)
                .max()
                .unwrap_or(1);
            let bottom = monitors
                .iter()
                .map(|m| m.position().y + m.size().height as i32)
                .max()
                .unwrap_or(1);
            let size = PhysicalSize::new((right - left) as u32, (bottom - top) as u32);
            let position = PhysicalPosition::new(left, top);
            let scale = monitor.scale_factor() as f32;
            (size, position, scale)
        };

        #[cfg(target_os = "macos")]
        let display =
            snipaste_platform::macos::select_capture_display().context("找不到截图显示器")?;
        #[cfg(target_os = "macos")]
        let (size, position, scale) = {
            let bounds = display.physical_bounds();
            (
                PhysicalSize::new(
                    bounds.width().round() as u32,
                    bounds.height().round() as u32,
                ),
                PhysicalPosition::new(bounds.min.x.round() as i32, bounds.min.y.round() as i32),
                display.scale as f32,
            )
        };

        log::info!(
            "主显示器 {}×{} @ ({}, {})，缩放 {}",
            size.width,
            size.height,
            position.x,
            position.y,
            scale
        );

        let attrs = Window::default_attributes()
            .with_title("Snipaste-RS")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            // 冻结桌面覆盖整屏，使用不透明表面避免透明窗口的合成闪烁。
            .with_transparent(false)
            // 置顶：截图的时候不能被别的窗口盖住
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new(size.width, size.height))
            .with_position(PhysicalPosition::new(position.x, position.y));

        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::{OptionAsAlt, WindowAttributesExtMacOS};
            let bounds = display.logical_bounds();
            attrs
                .with_has_shadow(false)
                .with_accepts_first_mouse(true)
                .with_option_as_alt(OptionAsAlt::Both)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    bounds.width(),
                    bounds.height(),
                ))
                .with_position(winit::dpi::LogicalPosition::new(bounds.min.x, bounds.min.y))
        };

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .context("创建遮罩窗口失败")?,
        );

        // 在隐藏窗口中准备冻结底图和首帧，之后再显示。
        window.set_visible(false);
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                snipaste_platform::window::disable_transitions(handle.hwnd.get());
            }
            #[cfg(target_os = "macos")]
            if let RawWindowHandle::AppKit(handle) = handle.as_raw() {
                // SAFETY: this live winit NSView is configured on the main thread.
                unsafe {
                    snipaste_platform::macos::configure_window(handle.ns_view.as_ptr(), true)
                };
            }
        }

        let viewport = Viewport::new(Pos::new(position.x as f32, position.y as f32), scale);

        let surface = if let Some(gfx) = self.gfx.as_ref() {
            gfx.instance.create_surface(window.clone())?
        } else {
            let (gfx, surface) = pollster::block_on(GpuContext::new(&window))?;
            self.gfx = Some(gfx);
            surface
        };
        let overlay = SurfaceWindow::new(
            self.gfx.as_ref().unwrap(),
            &self.egui_ctx,
            window,
            surface,
            viewport,
            false,
        )?;

        // 记下可选区边界，供拖拽时钳制选区使用。
        let screen = Rect::from_points(
            viewport.origin,
            Pos::new(
                viewport.origin.x + size.width as f32,
                viewport.origin.y + size.height as f32,
            ),
        );
        self.editor.set_screen_bounds(screen);
        self.screen_bounds = Some(screen);

        self.overlay = Some(overlay);
        self.egui_ctx.input_mut(|input| {
            input.max_texture_side = self
                .gfx
                .as_ref()
                .unwrap()
                .device
                .limits()
                .max_texture_dimension_2d as usize;
        });

        Ok(())
    }

    // ------------------------------------------------------------ 呼出截图

    /// 遮罩窗口覆盖的 VSS 矩形（物理像素）。
    ///
    /// 直接从 `Viewport::origin` 与窗口物理尺寸构造，**不经过逻辑点** ——
    /// 少一次换算就少一次把 `scale` 用反的机会。
    fn capture_screen(&self) -> Option<Rect> {
        let Some(o) = self.overlay.as_ref() else {
            return self.screen_bounds;
        };
        let size = o.size();
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let origin = o.viewport.origin;
        Some(Rect::from_points(
            origin,
            Pos::new(origin.x + size.width as f32, origin.y + size.height as f32),
        ))
    }

    /// 开始一次截图：显示遮罩、复位选区、进入窗口吸附状态。
    ///
    /// **这是唯一的呼出入口**（热键、托盘菜单、冒烟模式都走它）。启动流程
    /// 刻意不碰它，从而保证"打开程序不会自动截图"。
    fn start_capture(&mut self, event_loop: &ActiveEventLoop) {
        if self.visible {
            self.set_overlay_visible(false);
        }
        #[cfg(target_os = "macos")]
        if !snipaste_platform::macos::screen_permission(true) {
            snipaste_platform::dialog::show_information("需要屏幕录制权限",
                "请在「系统设置 → 隐私与安全性 → 屏幕与系统音频录制」中允许 Snipaste-RS。\n授权后从菜单栏退出应用，再重新打开。\n菜单栏中的「屏幕录制权限设置…」可以直接打开该页面。");
            return;
        }
        #[cfg(target_os = "macos")]
        snipaste_platform::macos::begin_capture();
        if let Err(err) = self.ensure_graphics(event_loop) {
            log::error!("截图界面初始化失败：{err:#}");
            self.set_overlay_visible(false);
            return;
        }
        let Some(screen) = self.capture_screen() else {
            log::warn!("遮罩窗口尚未就绪，无法开始截图");
            return;
        };

        // 先冻结桌面；在隐藏窗口中绘制首帧时，UI 按当前指针预选窗口。
        let shot = match snipaste_capture::capture(screen) {
            Ok(shot) => shot,
            Err(err) => {
                log::error!("无法截图：{err}");
                self.set_overlay_visible(false);
                snipaste_platform::dialog::show_information("无法截图", &err.to_string());
                return;
            }
        };
        self.editor.reset_session(screen, !snap_disabled());
        self.egui_ctx
            .input_mut(|input| input.pointer = Default::default());
        self.qr_request += 1;
        self.capture_texture = Some(self.egui_ctx.load_texture(
            "capture.background",
            export::color_image(&shot.image),
            egui::TextureOptions::NEAREST,
        ));
        self.snapshot = Some(shot);

        self.set_overlay_visible(true);
        if self.visible {
            log::info!("已进入截图（移动鼠标选择窗口，拖拽可自由框选）");
        }
    }

    fn set_overlay_visible(&mut self, visible: bool) {
        if visible {
            self.graphics_release_deadline = None;
            // 隐藏时先提交本次冻结桌面，不能让窗口先显示上一张截图或空交换链。
            let prepared = self.frames_rendered;
            self.render_frame();
            if self.frames_rendered == prepared {
                self.render_frame();
            }
            if self.frames_rendered == prepared {
                log::error!("截图首帧未就绪，保持窗口隐藏");
                self.set_overlay_visible(false);
                return;
            }
            if let Some(gfx) = self.gfx.as_ref() {
                if let Err(err) = gfx.device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(2)),
                }) {
                    log::error!("截图首帧呈现失败：{err}");
                    self.set_overlay_visible(false);
                    return;
                }
            }
        }
        self.visible = visible;
        if let Some(o) = self.overlay.as_ref() {
            o.window.set_visible(visible);
            snipaste_platform::window::flush_compositor();
            if visible {
                o.window.request_redraw();
                o.window.focus_window();
            }
        }
        if !visible {
            // 收起时顺手停掉吸附：否则下次呼出的第一帧会带着上一次的
            // 指针位置去做一次无意义的窗口查询。
            self.editor.disarm_snap();
            self.qr_request += 1;
            self.editor.qr = None;
            self.editor.text = None;
            self.snapshot = None;
            self.capture_texture = None;
            snipaste_render::canvas::clear_image_cache(&self.egui_ctx);
            // 整个会话销毁：交换链、GPU 纹理、图集和历史不再等下一次重绘才释放。
            self.overlay = None;
            #[cfg(target_os = "macos")]
            snipaste_platform::macos::end_capture();
            self.egui_ctx = egui::Context::default();
            if let Some(screen) = self.screen_bounds {
                self.editor.reset_session(screen, false);
            }
            self.repaint_deadline = None;
            self.schedule_graphics_release();
        }
    }

    fn schedule_graphics_release(&mut self) {
        self.graphics_release_deadline =
            if !self.visible && self.pins.is_empty() && self.gfx.is_some() {
                Some(std::time::Instant::now() + std::time::Duration::from_secs(3))
            } else {
                None
            };
    }

    fn release_idle_graphics(&mut self) {
        if self.visible || !self.pins.is_empty() {
            return;
        }
        self.overlay = None;
        self.egui_ctx = egui::Context::default();
        if let Some(gfx) = self.gfx.take() {
            let _ = gfx.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(2)),
            });
            drop(gfx);
            log::info!("空闲图形资源已释放");
        }
        self.graphics_release_deadline = None;
    }

    // ------------------------------------------------------------ 渲染

    fn redraw(&mut self) {
        if !self.visible {
            return;
        }
        let cmds = match self.render_frame() {
            Some(cmds) => cmds,
            None => return,
        };
        for cmd in cmds {
            self.handle_command(cmd);
        }
    }

    /// 渲染一帧，返回界面产生的命令。
    ///
    /// 单独拆一个函数是为了把借用收窄：闭包里要可变借用 `editor`，
    /// 渲染部分又要可变借用 `overlay`，如果都写在 `redraw` 里，
    /// 之后还要 `self.handle_command(...)` 就会和借用打架。
    fn render_frame(&mut self) -> Option<Vec<Command>> {
        let gfx = self.gfx.as_ref()?;
        let overlay = self.overlay.as_mut()?;

        let size = overlay.size();
        if size.width == 0 || size.height == 0 {
            return None;
        }

        let mut raw_input = overlay.egui.take_egui_input(&overlay.window);
        let view = overlay.viewport;
        if !self.visible {
            // 首帧直接使用当前屏幕指针，让窗口预选与放大镜在显示前落位。
            raw_input.events.clear();
            if let Some(pointer) = snipaste_platform::window::cursor_position() {
                raw_input
                    .events
                    .push(egui::Event::PointerMoved(view.to_points(pointer)));
            }
        }
        let hotkey_label = self.hotkey_label.as_str();
        let editor = &mut self.editor;
        let background = self
            .snapshot
            .as_ref()
            .zip(self.capture_texture.as_ref())
            .map(|(snapshot, texture)| ui::Background { snapshot, texture });

        let mut cmds = Vec::new();
        let mut full = self.egui_ctx.run_ui(raw_input, |ui| {
            cmds = ui::draw(ui, editor, view, background, hotkey_label);
        });
        self.repaint_deadline = full
            .viewport_output
            .values()
            .filter_map(|v| std::time::Instant::now().checked_add(v.repaint_delay))
            .min();

        // egui 的平台输出：光标形状、剪贴板、**以及 IME 开关与光标区域**。
        // 这一点很关键 —— egui-winit 会在这里自动调 `set_ime_allowed` /
        // `set_ime_cursor_area`，所以中文输入法不需要我们额外接线。
        overlay
            .egui
            .handle_platform_output(&overlay.window, full.platform_output);

        let pixels_per_point = full.pixels_per_point;
        let clipped = self.egui_ctx.tessellate(full.shapes, pixels_per_point);

        // 把纹理增量从 `full` 里**取出来**单独持有（`mem::take` 会把
        // `full.textures_delta` 换成空值，于是 drop `full` 时不会触发
        // epaint 的"有增量未处理"断言）。
        //
        // 这个断言正是用来揪出"忘了上传字体纹理"这类 bug 的（debug 构建下
        // 直接 panic —— 第一版就踩到了）。取出来之后，无论下面走哪条提前
        // 结束的分支，都能在函数出口统一 `clear()`。
        let mut textures = std::mem::take(&mut full.textures_delta);

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.width, size.height],
            pixels_per_point,
        };

        // egui 0.36 起，一张纹理的一次更新可以带**多个** ImageDelta
        // （`textures_delta.set` 的值是 SmallVec），必须逐个应用。
        for (id, deltas) in &textures.set {
            for delta in deltas {
                overlay
                    .painter
                    .update_texture(&gfx.device, &gfx.queue, *id, delta);
            }
        }

        let mut encoder = gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui"),
            });
        let user_cmds = overlay.painter.update_buffers(
            &gfx.device,
            &gfx.queue,
            &mut encoder,
            &clipped,
            &screen_desc,
        );

        // wgpu 30 起 `get_current_texture` 返回的是 `CurrentSurfaceTexture`
        // 而不是 `Result`：它把"拿得到帧但状态不理想"（Suboptimal）和
        // "这一帧拿不到但不算错"（Timeout / Occluded）都表达成了正常分支。
        //
        // 这里刻意用 `Option` 而不是提前 `return` —— 出口处还要释放
        // 已废弃的 egui 纹理并清空 `textures`，不能中途溜走。
        let frame = match overlay.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => Some(f),
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                // 帧是好的、能画，只是配置已不匹配（例如刚换过显示器）。
                // 先重配，这样下一帧就恢复正常。
                log::debug!("交换链进入 Suboptimal，重新配置表面");
                overlay.surface.configure(&gfx.device, &overlay.config);
                Some(f)
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                // 超时 / 窗口被遮挡：跳过这一帧，不是错误，也不要重配
                log::trace!("交换链本帧不可用（Timeout / Occluded），跳过");
                None
            }
            _ => {
                // Outdated / Lost / Validation：交换链确实失效了，重配一次
                log::debug!("交换链失效，重新配置表面");
                overlay.surface.configure(&gfx.device, &overlay.config);
                None
            }
        };

        if let Some(frame) = frame {
            let view_tex = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view_tex,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(overlay.clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    // wgpu 30 新增：多视图渲染的层掩码，本项目用不到
                    multiview_mask: None,
                });
                // `forget_lifetime` 把 `RenderPass<'_>` 变成 `RenderPass<'static>`，
                // 这是 egui-wgpu 要求的类型。放进独立作用域，保证在
                // `encoder.finish()` 之前把借还回去。
                let mut pass = pass.forget_lifetime();
                overlay.painter.render(&mut pass, &clipped, &screen_desc);
            }

            gfx.queue.submit(
                user_cmds
                    .into_iter()
                    .chain(std::iter::once(encoder.finish())),
            );
            overlay.window.pre_present_notify();
            // wgpu 30 起 `present` 从 `SurfaceTexture` 移到了 `Queue` 上：
            // 呈现本质上是一次队列操作，放在队列上语义更顺。
            gfx.queue.present(frame);
            self.frames_rendered += 1;
        }
        // 拿不到帧时 `user_cmds` 与 `encoder` 被直接丢弃 —— 这是有意的，
        // 本帧本来就不呈现。

        // 释放 egui 声明不再使用的纹理（字体重建、图集重排时会走到这里）
        for id in &textures.free {
            overlay.painter.free_texture(id);
        }
        textures.clear();

        Some(cmds)
    }

    // ------------------------------------------------------------ 命令

    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Undo => {
                if !self.editor.undo() {
                    self.editor.note("没有可撤销的操作");
                }
            }
            Command::Redo => {
                if !self.editor.redo() {
                    self.editor.note("没有可重做的操作");
                }
            }
            Command::ClearAll => {
                if !self.editor.clear_annotations() {
                    self.editor.note("画布已经是空的");
                }
            }
            Command::Cancel => self.set_overlay_visible(false),
            Command::CopyAndExit => {
                match self.selected_image().and_then(|image| {
                    self.clipboard()?
                        .set_image(image.width(), image.height(), image.as_raw())?;
                    log::info!(
                        "完成截图：{}×{} 图像已写入系统剪贴板",
                        image.width(),
                        image.height()
                    );
                    Ok(())
                }) {
                    Ok(()) => self.set_overlay_visible(false),
                    Err(err) => self.editor.note(format!("复制失败，请重试：{err:#}")),
                }
            }
            Command::SaveAs => match self.selected_image() {
                Ok(image) => {
                    if let Some(o) = self.overlay.as_ref() {
                        o.window.set_visible(false);
                    }
                    match save_image(&image) {
                        Ok(true) => self.set_overlay_visible(false),
                        Ok(false) => self.set_overlay_visible(true),
                        Err(err) => {
                            self.set_overlay_visible(true);
                            self.editor.note(format!("保存失败：{err:#}"));
                        }
                    }
                }
                Err(err) => self.editor.note(format!("无法保存：{err:#}")),
            },
            Command::Pin => match self.selected_image() {
                Ok(image) => {
                    let origin = self
                        .editor
                        .doc
                        .selection
                        .map(|r| r.min)
                        .unwrap_or(Pos::ZERO);
                    let _ = self.proxy.send_event(UserEvent::PinImage { image, origin });
                }
                Err(err) => self.editor.note(format!("无法贴图：{err:#}")),
            },
            Command::RecognizeQr => {
                let crop = self
                    .snapshot
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("没有截图底图"))
                    .and_then(|shot| {
                        shot.crop(
                            self.editor
                                .doc
                                .selection
                                .context("请先选择二维码所在区域")?,
                        )
                        .map_err(Into::into)
                    });
                match crop {
                    Ok(crop) => {
                        self.editor.commit_text();
                        self.editor.disarm_snap();
                        self.qr_request += 1;
                        let request = self.qr_request;
                        let proxy = self.proxy.clone();
                        self.editor.qr = Some(QrResult::Pending);
                        std::thread::spawn(move || {
                            let result = qr::recognize(&crop.image);
                            let _ = proxy.send_event(UserEvent::QrResult { request, result });
                        });
                    }
                    Err(err) => self.editor.note(format!("无法识别：{err:#}")),
                }
            }
            Command::CopyQr => {
                if let Some(QrResult::Found(items)) = self.editor.qr.as_ref() {
                    let content = items.join("\n");
                    match self
                        .clipboard()
                        .and_then(|clip| clip.set_text(&content).map_err(Into::into))
                    {
                        Ok(()) => self.editor.note("二维码内容已复制"),
                        Err(err) => self.editor.note(format!("复制失败：{err:#}")),
                    }
                }
            }
            Command::CopyColor { color, format } => {
                let content = format.value(color);
                match self
                    .clipboard()
                    .and_then(|clip| clip.set_text(&content).map_err(Into::into))
                {
                    Ok(()) => self.editor.note(format!("已复制颜色：{content}")),
                    Err(err) => self.editor.note(format!("复制颜色失败：{err:#}")),
                }
            }
        }
        self.request_repaint();
    }

    fn request_repaint(&self) {
        if let Some(o) = self.overlay.as_ref() {
            if self.visible {
                o.window.request_redraw();
            }
        }
    }

    fn clipboard(&mut self) -> Result<&mut snipaste_platform::clipboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Some(snipaste_platform::clipboard::Clipboard::new()?);
        }
        Ok(self.clipboard.as_mut().expect("剪贴板已初始化"))
    }

    fn selected_image(&mut self) -> Result<image::RgbaImage> {
        self.editor.commit_text();
        self.editor.cancel_drag();
        let background = self
            .overlay
            .as_ref()
            .zip(self.capture_texture.as_ref())
            .and_then(|(overlay, texture)| overlay.painter.texture(&texture.id()))
            .and_then(|texture| texture.texture.as_ref());
        export::compose_with_texture(
            self.gfx.as_ref().context("图形设备未就绪")?,
            self.snapshot.as_ref().context("截图底图未就绪")?,
            &self.editor.doc,
            background,
        )
    }

    fn paste_clipboard(&mut self, event_loop: &ActiveEventLoop) {
        if self.visible {
            self.handle_command(Command::Pin);
            return;
        }
        match self
            .clipboard()
            .and_then(|clip| clip.image().map_err(Into::into))
        {
            Ok((width, height, rgba)) => {
                if let Some(image) = image::RgbaImage::from_raw(width, height, rgba) {
                    #[cfg(not(target_os = "macos"))]
                    let origin = event_loop
                        .primary_monitor()
                        .map(|m| {
                            let p = m.position();
                            let s = m.size();
                            Pos::new(
                                p.x as f32 + (s.width as f32 - width as f32) * 0.5,
                                p.y as f32 + (s.height as f32 - height as f32) * 0.5,
                            )
                        })
                        .unwrap_or(Pos::ZERO);
                    #[cfg(target_os = "macos")]
                    let origin = snipaste_platform::macos::select_capture_display()
                        .map(|display| {
                            let bounds = display.physical_bounds();
                            bounds.min
                                + Pos::new(
                                    (bounds.width() - width as f32) * 0.5,
                                    (bounds.height() - height as f32) * 0.5,
                                )
                        })
                        .unwrap_or(Pos::ZERO);
                    self.open_pin(event_loop, image, origin);
                }
            }
            Err(err) => log::warn!("剪贴板中没有可贴出的图片：{err:#}"),
        }
    }

    fn open_pin(&mut self, event_loop: &ActiveEventLoop, image: image::RgbaImage, origin: Pos) {
        if let Err(err) = self.ensure_graphics(event_loop) {
            log::error!("贴图初始化失败：{err:#}");
            self.set_overlay_visible(false);
            return;
        }
        let result = self
            .gfx
            .as_ref()
            .context("图形设备未就绪")
            .and_then(|gfx| PinWindow::new(event_loop, gfx, image, origin));
        match result {
            Ok(pin) => {
                self.pins.push(pin);
                self.set_overlay_visible(false);
            }
            Err(err) => {
                self.editor.note(format!("创建贴图失败：{err:#}"));
                if !self.visible {
                    self.set_overlay_visible(false);
                }
            }
        }
    }

    // ------------------------------------------------------------ 键盘

    fn handle_color_picker_key(&mut self, key: &Key, repeat: bool) -> bool {
        if !self.visible
            || self.modifiers.control_key()
            || self.modifiers.super_key()
            || !self.editor.show_magnifier(self.modifiers.alt_key())
            || self.egui_ctx.egui_wants_keyboard_input()
        {
            return false;
        }
        let toggle = *key == Key::Named(NamedKey::Shift);
        let copy = matches!(key, Key::Character(s) if s.eq_ignore_ascii_case("c"));
        if !toggle && !copy {
            return false;
        }
        // 取最新物理指针位置，避免 C 发生在下一次重绘之前时复制上一个像素。
        let Some(sample) = self.snapshot.as_ref().and_then(|snapshot| {
            snipaste_platform::window::cursor_position()
                .or_else(|| {
                    self.overlay.as_ref().and_then(|o| {
                        self.egui_ctx
                            .input(|i| i.pointer.hover_pos())
                            .map(|p| o.viewport.to_vss(p))
                    })
                })
                .and_then(|p| color_picker::sample(snapshot, p))
        }) else {
            return false;
        };
        if !repeat {
            if toggle {
                self.editor.color_format.toggle();
            } else {
                self.handle_command(Command::CopyColor {
                    color: sample.rgb,
                    format: self.editor.color_format,
                });
            }
            self.request_repaint();
        }
        true
    }

    fn handle_key(&mut self, key: &KeyEvent) {
        if key.state != ElementState::Pressed || !self.visible {
            return;
        }
        // 从 egui 拿修饰键状态：它已经处理过平台差异（macOS 的 Cmd ↔ Ctrl）
        let command = self.modifiers.control_key() || self.modifiers.super_key();
        let shift = self.modifiers.shift_key();
        if self.editor.qr.is_some() {
            if key.logical_key == Key::Named(NamedKey::Escape) {
                self.editor.qr = None;
                self.qr_request += 1;
                self.request_repaint();
            }
            return;
        }
        if self.editor.text.is_some() || self.egui_ctx.egui_wants_keyboard_input() {
            if key.logical_key == Key::Named(NamedKey::Escape) {
                self.editor.text = None;
                self.egui_ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.request_repaint();
            }
            return;
        }

        if self.handle_color_picker_key(&key.logical_key, key.repeat) {
            return;
        }
        match &key.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.editor.is_dragging() {
                    // 正在画 → 只放弃这一笔
                    self.editor.cancel_drag();
                } else {
                    // 没在画 → 收起遮罩
                    self.set_overlay_visible(false);
                    return;
                }
            }
            Key::Named(NamedKey::Tab) => {
                let next = self.editor.doc.tool.next();
                self.editor.set_tool(next);
            }
            Key::Named(NamedKey::Enter) => {
                self.handle_command(Command::CopyAndExit);
            }
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => self.editor.delete_selected(),
            Key::Character(s) if command => match s.to_ascii_lowercase().as_str() {
                "z" if shift => self.handle_command(Command::Redo),
                "z" => self.handle_command(Command::Undo),
                "y" => self.handle_command(Command::Redo),
                "s" => self.handle_command(Command::SaveAs),
                "c" => self.handle_command(Command::CopyAndExit),
                "a" => {
                    if let Some(screen) = self.capture_screen() {
                        self.editor.select_full_screen(screen);
                        self.editor.disarm_snap();
                    }
                }
                _ => {}
            },
            Key::Named(
                NamedKey::ArrowLeft
                | NamedKey::ArrowRight
                | NamedKey::ArrowUp
                | NamedKey::ArrowDown,
            ) => {
                let step = if command { 10.0 } else { 1.0 };
                let (dx, dy) = match key.logical_key {
                    Key::Named(NamedKey::ArrowLeft) => (-step, 0.0),
                    Key::Named(NamedKey::ArrowRight) => (step, 0.0),
                    Key::Named(NamedKey::ArrowUp) => (0.0, -step),
                    _ => (0.0, step),
                };
                self.editor.nudge_selection(dx, dy, shift);
            }
            Key::Character(s) if !self.modifiers.alt_key() => {
                // 单字母切工具。注意这里**不拦**输入法正在合成的按键 ——
                // 合成期间 egui 会把事件标成 `IME`，且此时工具切换本来
                // 也不该发生。
                if let Some(ch) = s.chars().next() {
                    let upper = ch.to_ascii_uppercase();
                    if upper == 'Q' {
                        self.handle_command(Command::RecognizeQr);
                    } else if let Some(tool) =
                        Tool::ALL.iter().find(|t| t.shortcut() == Some(upper))
                    {
                        self.editor.set_tool(*tool);
                    }
                }
            }
            _ => {}
        }

        self.request_repaint();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(err) = self.ensure_started(event_loop) {
            log::error!("启动失败：{err:#}");
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = std::time::Instant::now();
        if self
            .graphics_release_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.release_idle_graphics();
        }
        if self
            .memory_profile
            .as_ref()
            .is_some_and(|p| p.deadline <= now)
        {
            let mut profile = self.memory_profile.take().unwrap();
            if let Err(err) = self.profile_memory_step(event_loop, &mut profile) {
                log::error!("内存验收失败：{err:#}");
                event_loop.exit();
                return;
            }
            if profile.step == 12 {
                log::info!("内存验收完成");
                event_loop.exit();
                return;
            }
            let delay = if profile.step % 4 == 3 { 5 } else { 1 };
            profile.step += 1;
            profile.deadline = std::time::Instant::now() + std::time::Duration::from_secs(delay);
            self.memory_profile = Some(profile);
        }
        if self.visible
            && self
                .repaint_deadline
                .is_some_and(|deadline| deadline <= now)
        {
            self.repaint_deadline = None;
            self.request_repaint();
        }
        // 冒烟模式：`Wait` 会让线程彻底睡过去，必须显式设一个唤醒时间点。
        if let Some(deadline) = self.smoke_deadline {
            let now = std::time::Instant::now();
            if now >= deadline {
                log::info!("冒烟模式结束：共呈现 {} 帧，正常退出", self.frames_rendered);
                // 清掉截止时刻：`exit()` 之后事件循环还可能再转一圈，
                // 不清的话这条日志会打两遍。
                self.smoke_deadline = None;
                event_loop.exit();
                return;
            }
        }
        let deadline = self
            .smoke_deadline
            .into_iter()
            .chain(self.repaint_deadline.filter(|_| self.visible))
            .chain(self.memory_profile.as_ref().map(|p| p.deadline))
            .chain(self.graphics_release_deadline)
            .min();
        event_loop.set_control_flow(match deadline {
            Some(deadline) => winit::event_loop::ControlFlow::WaitUntil(deadline),
            None => winit::event_loop::ControlFlow::Wait,
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Hotkey(id) => {
                let action = self.hotkeys.as_ref().and_then(|h| h.action_for(id));
                match action {
                    Some(Action::Capture) => {
                        if self.visible {
                            // 遮罩已经开着 → 再按一次收起，与 Snipaste 的 F1 行为一致
                            self.set_overlay_visible(false);
                            log::info!("已收起截图遮罩");
                        } else {
                            self.start_capture(event_loop);
                            self.request_repaint();
                        }
                    }
                    Some(Action::PasteLast) => self.paste_clipboard(event_loop),
                    None => log::debug!("未映射的热键 id {id}"),
                }
            }
            UserEvent::Menu(action) => self.handle_menu(event_loop, action),
            UserEvent::QrResult { request, result } => {
                if self.visible && self.qr_request == request && self.editor.qr.is_some() {
                    self.editor.qr = Some(match result {
                        Ok(items) => QrResult::Found(items),
                        Err(err) => QrResult::Error(err),
                    });
                    self.request_repaint();
                }
            }
            UserEvent::PinImage { image, origin } => self.open_pin(event_loop, image, origin),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(index) = self.pins.iter().position(|p| p.surface.window.id() == id) {
            let command = self
                .gfx
                .as_ref()
                .and_then(|gfx| self.pins[index].event(gfx, &event));
            match command {
                Some(PinCommand::Close) => {
                    self.pins.remove(index);
                    self.schedule_graphics_release();
                }
                Some(PinCommand::Copy) => {
                    if let Err(err) = self.copy_pin(index) {
                        log::warn!("复制贴图失败：{err:#}");
                    }
                }
                Some(PinCommand::Save) => {
                    if let Err(err) = save_image(&self.pins[index].image) {
                        log::warn!("保存贴图失败：{err:#}");
                    }
                }
                None => {}
            }
            return;
        }
        // 只处理遮罩窗口的事件
        if self.overlay.as_ref().map(|o| o.window.id()) != Some(id) {
            return;
        }

        // ① 先让 egui 消费事件（它要维护自己的输入状态）
        let wants_repaint = {
            let overlay = match self.overlay.as_mut() {
                Some(o) => o,
                None => return,
            };
            let resp = overlay.egui.on_window_event(&overlay.window, &event);
            resp.repaint
        };

        // ② 我们自己的处理
        match event {
            WindowEvent::CloseRequested => self.set_overlay_visible(false),
            WindowEvent::Resized(size) => {
                if let (Some(o), Some(g)) = (self.overlay.as_mut(), self.gfx.as_ref()) {
                    o.resize(g, size);
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(o) = self.overlay.as_mut() {
                    o.viewport.scale = scale_factor as f32;
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) => {
                self.modifiers = winit::keyboard::ModifiersState::empty()
            }
            WindowEvent::Destroyed => event_loop.exit(),
            _ => {}
        }

        if wants_repaint {
            self.request_repaint();
        }
    }
}

impl App {
    fn copy_pin(&mut self, index: usize) -> Result<()> {
        self.clipboard()?;
        let image = &self.pins[index].image;
        self.clipboard.as_mut().unwrap().set_image(
            image.width(),
            image.height(),
            image.as_raw(),
        )?;
        Ok(())
    }
    fn profile_memory_step(
        &mut self,
        event_loop: &ActiveEventLoop,
        profile: &mut crate::memory_profile::Profile,
    ) -> Result<()> {
        let cycle = profile.step.div_ceil(4);
        let mut capture_ms = 0;
        let stage = match profile.step % 4 {
            0 => {
                if profile.step == 0 {
                    "startup"
                } else {
                    "idle"
                }
            }
            1 => {
                let start = std::time::Instant::now();
                self.start_capture(event_loop);
                anyhow::ensure!(self.visible, "无法开始内存验收截图");
                capture_ms = start.elapsed().as_millis();
                "capture"
            }
            2 => {
                let screen = self.capture_screen().context("缺少屏幕")?;
                self.editor.set_selection(Some(screen));
                self.editor.set_tool(Tool::Mosaic);
                for block in [8, 16, 24, 32] {
                    self.editor.style.mosaic_block = block;
                    self.editor.begin(screen.min + Pos::new(120.0, 120.0));
                    self.editor.extend(screen.min + Pos::new(420.0, 320.0));
                    self.editor.finish(screen.min + Pos::new(420.0, 320.0));
                    self.render_frame();
                }
                "mosaic"
            }
            _ => {
                let image = self.selected_image()?;
                anyhow::ensure!(!image.is_empty(), "内存验收导出失败");
                drop(image);
                "export"
            }
        };
        if profile.step.is_multiple_of(4) {
            anyhow::ensure!(
                self.gfx.is_none() && self.overlay.is_none(),
                "待机仍保留图形资源"
            );
        }
        profile.record(
            &format!("{stage}-{cycle}"),
            capture_ms,
            self.gfx.is_some(),
            self.overlay.is_some(),
        )?;
        if profile.step % 4 == 3 {
            self.set_overlay_visible(false);
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn verify_native(&mut self, event_loop: &ActiveEventLoop, dir: &std::path::Path) -> Result<()> {
        use crate::acceptance;
        std::fs::create_dir_all(dir)?;
        // 原生捕获必须读到不透明像素，不能用透明遮罩伪装截图。
        let capture = snipaste_capture::capture(self.capture_screen().context("缺少屏幕")?)?;
        anyhow::ensure!(
            capture.image.pixels().all(|p| p[3] == 255),
            "屏幕捕获透明度错误"
        );
        let saved_image = self.clipboard()?.image().ok();
        let saved_text = self.clipboard()?.text().ok();
        let result = (|| -> Result<()> {
            acceptance::verify_color_ui(self.gfx.as_ref().context("缺少 GPU")?, dir)?;
            self.verify_color_picker_keys()?;
            acceptance::verify_effects(self.gfx.as_ref().context("缺少 GPU")?, dir)?;
            self.snapshot = Some(acceptance::fixture()?);
            self.editor = acceptance::editor();
            acceptance::render_ui(
                self.gfx.as_ref().context("缺少 GPU")?,
                self.snapshot.as_ref().context("缺少底图")?,
                &mut self.editor,
                &dir.join("capture-ui.png"),
            )?;
            let expected = self.selected_image()?;
            anyhow::ensure!(expected.dimensions() == (760, 450), "导出尺寸错误");
            anyhow::ensure!(
                expected.get_pixel(60, 60)[0] > 200 && expected.get_pixel(60, 60)[1] < 100,
                "矩形标注没有被导出"
            );
            expected.save(dir.join("confirmed.png"))?;
            let preview = image::open(dir.join("capture-ui.png"))?.to_rgba8();
            for (x, y) in [(240, 460), (260, 470), (390, 480)] {
                anyhow::ensure!(
                    preview.get_pixel(x, y) == expected.get_pixel(x - 160, y - 120),
                    "马赛克预览与确认图片不一致"
                );
            }
            let shot = self.snapshot.as_ref().context("缺少底图")?;
            let crop = shot.crop(self.editor.doc.selection.context("缺少选区")?)?;
            let codes = qr::recognize(&crop.image).map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                codes == vec!["https://example.com/snipaste-test"],
                "二维码结果错误"
            );
            self.editor.qr = Some(QrResult::Found(codes));
            acceptance::render_ui(
                self.gfx.as_ref().context("缺少 GPU")?,
                shot,
                &mut self.editor,
                &dir.join("qr-ui.png"),
            )?;
            self.visible = true;
            self.handle_command(Command::CopyQr);
            anyhow::ensure!(
                self.clipboard()?.text()? == "https://example.com/snipaste-test",
                "二维码复制失败"
            );
            self.handle_command(Command::CopyAndExit);
            anyhow::ensure!(!self.visible, "完成截图后遮罩未关闭");
            let (w, h, bytes) = self.clipboard()?.image()?;
            anyhow::ensure!(
                (w, h) == expected.dimensions() && bytes == *expected.as_raw(),
                "剪贴板图像与导出不一致"
            );
            self.open_pin(event_loop, expected, Pos::new(120.0, 120.0));
            anyhow::ensure!(!self.pins.is_empty(), "贴图窗口创建失败");
            if let Some(gfx) = self.gfx.as_ref() {
                self.pins[0].event(gfx, &WindowEvent::RedrawRequested);
            }
            self.pins.clear();
            self.verify_transitions(event_loop)?;
            self.verify_resource_lifecycle(event_loop)?;
            std::fs::write(dir.join("result.txt"),"PASS: HEX/RGB magnifier, Shift format toggle, C color clipboard, Alt picker, pixel-aligned zoom, native capture, annotated export, compact rectangle/brush mosaic, partial/click/fast-sweep eraser, pixel-exact undo/redo, preview/export agreement, negative screen origin, QR decode, QR text clipboard, confirmed image clipboard, prepared capture transitions, cold capture restart, idle resource release, pin GPU lifetime\n")?;
            log::info!(
                "原生验收通过：HEX/RGB 取色、马赛克、局部擦除、显示切换、确认复制、二维码和贴图"
            );
            Ok(())
        })();
        if let Some((w, h, bytes)) = saved_image {
            let _ = self.clipboard()?.set_image(w, h, &bytes);
        } else if let Some(text) = saved_text {
            let _ = self.clipboard()?.set_text(&text);
        }
        result
    }

    #[cfg(debug_assertions)]
    fn verify_color_picker_keys(&mut self) -> Result<()> {
        use color_picker::ColorFormat;
        let screen = self.capture_screen().context("缺少屏幕")?;
        self.snapshot = Some(snipaste_capture::Snapshot {
            origin: screen.min,
            image: image::RgbaImage::from_pixel(
                screen.width() as u32,
                screen.height() as u32,
                image::Rgba([18, 52, 86, 255]),
            ),
        });
        self.editor.reset_session(screen, true);
        self.editor.color_format = ColorFormat::Hex;
        self.visible = true;
        self.modifiers = winit::keyboard::ModifiersState::empty();
        let copy = Key::Character("c".into());
        let shift = Key::Named(NamedKey::Shift);
        anyhow::ensure!(self.handle_color_picker_key(&copy, false), "C 未触发取色");
        anyhow::ensure!(self.clipboard()?.text()? == "#123456", "HEX 颜色复制错误");
        self.modifiers = winit::keyboard::ModifiersState::SHIFT;
        anyhow::ensure!(
            self.handle_color_picker_key(&shift, false),
            "Shift 未切换格式"
        );
        anyhow::ensure!(self.editor.color_format == ColorFormat::Rgb, "未切换为 RGB");
        self.handle_color_picker_key(&shift, true);
        anyhow::ensure!(
            self.editor.color_format == ColorFormat::Rgb,
            "按键重复导致多次切换"
        );
        self.modifiers = winit::keyboard::ModifiersState::empty();
        self.handle_color_picker_key(&copy, false);
        anyhow::ensure!(
            self.clipboard()?.text()? == "18, 52, 86",
            "RGB 颜色复制错误"
        );
        anyhow::ensure!(self.visible, "复制颜色意外退出截图");
        self.modifiers = winit::keyboard::ModifiersState::CONTROL;
        anyhow::ensure!(
            !self.handle_color_picker_key(&copy, false),
            "取色抢占了 Ctrl+C 确认截图"
        );
        self.modifiers = winit::keyboard::ModifiersState::empty();
        self.editor.disarm_snap();
        anyhow::ensure!(
            !self.handle_color_picker_key(&copy, false),
            "无放大镜时仍复制颜色"
        );
        self.modifiers = winit::keyboard::ModifiersState::ALT;
        anyhow::ensure!(
            self.handle_color_picker_key(&copy, false),
            "框选后 Alt 取色未生效"
        );
        self.handle_color_picker_key(&shift, false);
        anyhow::ensure!(self.editor.color_format == ColorFormat::Hex, "未切回 HEX");
        self.editor.start_text(screen.center());
        anyhow::ensure!(
            !self.handle_color_picker_key(&copy, false),
            "文字输入被取色拦截"
        );
        self.modifiers = winit::keyboard::ModifiersState::empty();
        self.set_overlay_visible(false);
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn verify_resource_lifecycle(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        self.release_idle_graphics();
        anyhow::ensure!(
            self.gfx.is_none() && self.overlay.is_none() && self.snapshot.is_none(),
            "空闲资源未释放"
        );
        anyhow::ensure!(
            self.editor.doc.items.is_empty() && self.editor.history.undo_len() == 0,
            "结束会话后历史未释放"
        );
        self.paste_clipboard(event_loop);
        anyhow::ensure!(
            self.gfx.is_some() && self.overlay.is_none() && self.pins.len() == 1,
            "冷启动剪贴板贴图失败"
        );
        self.release_idle_graphics();
        let gfx = self.gfx.as_ref().context("仍有贴图时 GPU 被释放")?;
        self.pins[0].event(gfx, &WindowEvent::RedrawRequested);
        self.start_capture(event_loop);
        anyhow::ensure!(self.visible && self.pins.len() == 1, "贴图期间截图失败");
        self.set_overlay_visible(false);
        anyhow::ensure!(
            self.gfx.is_some() && self.pins.len() == 1,
            "收起截图影响了贴图"
        );
        self.pins.clear();
        self.schedule_graphics_release();
        self.release_idle_graphics();
        anyhow::ensure!(self.gfx.is_none(), "最后一张贴图关闭后 GPU 未释放");
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn verify_transitions(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        self.ensure_graphics(event_loop)?;
        let screen = self.capture_screen().context("缺少屏幕")?;
        for rgb in [[40u8, 120, 220], [220, 80, 40], [60, 200, 90]] {
            self.ensure_graphics(event_loop)?;
            let mut shot = snipaste_capture::Snapshot {
                origin: screen.min,
                image: image::RgbaImage::from_pixel(
                    screen.width() as u32,
                    screen.height() as u32,
                    image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
                ),
            };
            // A different color at the mirrored location catches vertical flips
            // in the macOS CGImage -> RGBA path; a uniform frame cannot do that.
            let mirrored_y = shot.image.height().saturating_sub(81);
            for y in mirrored_y.saturating_sub(8)..(mirrored_y + 9).min(shot.image.height()) {
                for x in 72..89.min(shot.image.width()) {
                    shot.image.put_pixel(
                        x,
                        y,
                        image::Rgba([255 - rgb[0], 255 - rgb[1], 255 - rgb[2], 255]),
                    );
                }
            }
            self.editor.reset_session(screen, false);
            self.capture_texture = Some(self.egui_ctx.load_texture(
                "transition.fixture",
                export::color_image(&shot.image),
                egui::TextureOptions::NEAREST,
            ));
            self.snapshot = Some(shot);
            let frames = self.frames_rendered;
            self.set_overlay_visible(true);
            anyhow::ensure!(
                self.visible && self.frames_rendered > frames,
                "显示前没有呈现首帧"
            );
            let shown = snipaste_capture::capture(screen)?;
            let got = shown.image.get_pixel(80, 80);
            let dim = snipaste_render::theme::DIM_OVERLAY.to_srgba_unmultiplied();
            for c in 0..3 {
                let want =
                    (rgb[c] as u32 * (255 - dim[3] as u32) + dim[c] as u32 * dim[3] as u32) / 255;
                anyhow::ensure!(
                    (got[c] as i32 - want as i32).abs() <= 2,
                    "首个可见帧包含旧图/空白：{got:?}, 期望通道 {c}={want}"
                );
            }
            self.handle_command(Command::Cancel);
            anyhow::ensure!(!self.visible && self.snapshot.is_none(), "取消后未关闭遮罩");
            anyhow::ensure!(self.overlay.is_none(), "取消后原生窗口仍然显示");
            self.release_idle_graphics();
        }
        self.start_capture(event_loop);
        anyhow::ensure!(self.visible, "实际截图进入失败");
        self.editor.set_selection(Some(Rect::new(
            screen.min.x + 50.0,
            screen.min.y + 50.0,
            120.0,
            80.0,
        )));
        self.handle_command(Command::CopyAndExit);
        anyhow::ensure!(
            !self.visible && self.clipboard()?.image()?.0 == 120,
            "确认退出/复制失败"
        );
        Ok(())
    }

    fn handle_menu(&mut self, event_loop: &ActiveEventLoop, action: MenuAction) {
        match action {
            MenuAction::Capture => {
                self.start_capture(event_loop);
            }
            MenuAction::Paste => self.paste_clipboard(event_loop),
            MenuAction::ToggleAutostart => {
                let enabled = self
                    .tray
                    .as_ref()
                    .map(|t| t.autostart.is_checked())
                    .unwrap_or(false);
                if let Err(err) = snipaste_platform::autostart::set_enabled(enabled) {
                    if let Some(tray) = self.tray.as_ref() {
                        tray.autostart.set_checked(!enabled);
                    }
                    snipaste_platform::dialog::show_information("开机自启设置失败", &err);
                }
            }
            MenuAction::PinHelp => {
                if let Some(overlay) = self.overlay.as_ref() {
                    overlay.window.set_visible(false);
                }
                let modifier = if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl+"
                };
                let paste = self
                    .hotkeys
                    .as_ref()
                    .map(|h| h.paste_label.as_str())
                    .unwrap_or("菜单栏贴图");
                snipaste_platform::dialog::show_information("Snipaste-RS 操作说明", &format!(
                    "{}：截图\n放大镜取色：Shift 切换 HEX / RGB，C 复制颜色\n框选后按住 Alt / Option：临时取色\nEnter / 勾号 / {modifier}C：确认复制，随后 {modifier}V 粘贴\nQ：识别选区二维码\n{modifier}S：保存截图\n{paste}：桌面贴图\n\n贴图操作\n拖动：移动\n滚轮：缩放\nShift+滚轮：透明度\n{modifier}C：复制\n右键：更多操作\nEsc / 双击：关闭", self.hotkey_label));
                if self.visible {
                    if let Some(overlay) = self.overlay.as_ref() {
                        overlay.window.set_visible(true);
                        overlay.window.focus_window();
                    }
                }
            }
            #[cfg(target_os = "macos")]
            MenuAction::ScreenPermission => {
                if self.visible {
                    self.set_overlay_visible(false);
                }
                snipaste_platform::macos::open_screen_settings();
            }
            MenuAction::Quit => event_loop.exit(),
        }
        self.request_repaint();
    }
}

/// 环境变量 `SNIPASTE_NO_SNAP`：关闭「默认选区吸附指针下的窗口」。
///
/// 置为 `1` / `true` / 任意非空非 `0` 的值即生效。两个用途：
///
/// 1. **自动化测试**（`target/e2e_drag.py`）。吸附探针会在指针移动时不断
///    改写选区，脚本无法把指针停到角柄上再按下去 —— 每走一步选区就变了。
/// 2. **给用户的逃生开关**。吸附是基于窗口枚举的启发式，遇到窗口边框异常
///    的程序（无边框全屏、某些游戏）会给出让人意外的选区，此时可以直接关掉。
fn snap_disabled() -> bool {
    match std::env::var("SNIPASTE_NO_SNAP") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

fn save_image(image: &image::RgbaImage) -> Result<bool> {
    let Some(path) = snipaste_platform::dialog::save_image_path().map_err(anyhow::Error::msg)?
    else {
        return Ok(false);
    };
    let format = image::ImageFormat::from_path(&path)?;
    if format == image::ImageFormat::Jpeg {
        use image::buffer::ConvertBuffer;
        let rgb: image::RgbImage = image.convert();
        rgb.save_with_format(&path, format)?;
    } else {
        image.save_with_format(&path, format)?;
    }
    log::info!("截图已保存：{}", path.display());
    Ok(true)
}
