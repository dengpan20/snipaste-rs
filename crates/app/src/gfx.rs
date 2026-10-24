//! GPU 上下文与窗口表面。
//!
//! ## 为什么 device 全局一份、surface 每窗口一份
//!
//! `wgpu::Device` / `Queue` 创建成本高（驱动初始化、管线编译），而且是
//! `Send + Sync` 的；`wgpu::Surface` 则**必须**与窗口一一对应。
//! 本项目多屏时会同时存在 N 个遮罩窗口加若干贴图窗口，所以按下面的边界切：
//!
//! ```text
//! GpuContext      instance / adapter / device / queue      全局 1 份
//! SurfaceWindow   surface / config / egui State / Renderer  每窗口 1 份
//! ```
//!
//! egui 的 `Renderer` 也按窗口分开 —— 它内部缓存了一个存窗口尺寸的 uniform
//! buffer，两个尺寸不同的窗口共用一份会让 uniform 每帧来回重写。

use std::sync::Arc;

use anyhow::{Context as _, Result};
use egui_wgpu::{Renderer, RendererOptions};
use winit::window::Window;

/// 全局 GPU 上下文。
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// 创建上下文，并顺带为 `window` 建立表面。
    ///
    /// 顺序是死的：`instance → surface → adapter → device`。
    /// adapter 必须声明 `compatible_surface`，否则可能选中一块**无法向该窗口
    /// 呈现**的适配器，直到第一次 `get_current_texture` 才炸。
    pub async fn new(window: &Arc<Window>) -> Result<(Self, wgpu::Surface<'static>)> {
        // 单个后端避免同时初始化多套驱动。Windows 优先系统原生 DX12，失败回退 Vulkan。
        #[cfg(windows)]
        match Self::with_backends(window, wgpu::Backends::DX12).await {
            Ok(context) => return Ok(context),
            Err(err) => log::warn!("DX12 初始化失败，尝试 Vulkan：{err:#}"),
        }
        let backends = if cfg!(windows) {
            wgpu::Backends::VULKAN
        } else if cfg!(target_os = "macos") {
            wgpu::Backends::METAL
        } else {
            wgpu::Backends::PRIMARY
        };
        Self::with_backends(window, backends).await
    }

    async fn with_backends(
        window: &Arc<Window>,
        backends: wgpu::Backends,
    ) -> Result<(Self, wgpu::Surface<'static>)> {
        // wgpu 30 起 `InstanceDescriptor` 不再实现 `Default`（它多了一个
        // "连接到哪个显示服务器"的字段，默认值语义不明确），要显式构造。
        // 本项目的目标平台是 Windows / macOS，display 字段用不上。
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        #[cfg(windows)]
        {
            // HWND swapchains only support opaque surfaces. DirectComposition
            // preserves the pin's transparent margin and soft shadow alpha.
            descriptor.backend_options.dx12.presentation_system =
                wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        let instance = wgpu::Instance::new(descriptor);

        let surface = instance
            .create_surface(window.clone())
            .context("为该窗口创建 wgpu 表面失败")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                // 优先独显：显存不计入进程 RSS，符合"占用内存小"的目标；
                // 只有核显的机器上 wgpu 会自动回退，不需要我们判断。
                power_preference: if cfg!(target_os = "macos") {
                    wgpu::PowerPreference::LowPower
                } else {
                    wgpu::PowerPreference::HighPerformance
                },
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("找不到可用于该窗口的 GPU 适配器")?;

        let info = adapter.get_info();
        log::info!(
            "wgpu 适配器：{}（{:?} / {:?}）",
            info.name,
            info.backend,
            info.device_type
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("snipaste"),
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d,
                    ..wgpu::Limits::default()
                },
                // 截图工具的纹理都是一次性的，MemoryUsage 提示驱动别做长驻优化
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await
            .context("请求 GPU 设备失败")?;

        Ok((
            Self {
                instance,
                adapter,
                device,
                queue,
            },
            surface,
        ))
    }

    /// 为该表面挑一个可用的合成模式。
    ///
    /// 遮罩窗口需要**真透明**（才能看见底下的屏幕内容），所以优先
    /// `PreMultiplied`（egui 输出的是预乘色，正好对应）。
    /// 拿不到就退回驱动给的第一项：界面会变成不透明，但仍能正常工作。
    pub fn pick_alpha_mode(&self, surface: &wgpu::Surface<'_>) -> wgpu::CompositeAlphaMode {
        let caps = surface.get_capabilities(&self.adapter);
        if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Opaque)
        }
    }
}

/// 一个窗口的交换链 + egui 集成状态。
pub struct SurfaceWindow {
    pub window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    /// egui 的 winit 适配层：把 winit 事件翻译成 egui 输入、把 egui 输出
    /// 翻译回 winit 调用（光标、剪贴板、**以及 IME**）。
    pub egui: egui_winit::State,
    /// egui 的 wgpu 绘制器，与窗口的 surface 格式绑定。
    pub painter: Renderer,
    /// 本窗口覆盖的显示器在 VSS 中的原点与缩放。
    pub viewport: snipaste_render::Viewport,
    /// 清屏色。贴图的透明边距和阴影需要全透明。
    pub clear: wgpu::Color,
}

impl SurfaceWindow {
    pub fn new(
        gfx: &GpuContext,
        egui_ctx: &egui::Context,
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        viewport: snipaste_render::Viewport,
        transparent: bool,
    ) -> Result<Self> {
        let size = window.inner_size();
        let caps = surface.get_capabilities(&gfx.adapter);
        let mut config = surface
            .get_default_config(&gfx.adapter, size.width.max(1), size.height.max(1))
            .context("该适配器不支持向此窗口呈现")?;

        // egui 自己负责 gamma 处理，期望交换链是**非 sRGB** 格式。
        // 驱动默认给的多半是 `Bgra8UnormSrgb`，直接用不仅会让 egui-wgpu
        // 每次启动都警告一次，颜色也会整体偏亮。
        if let Some(best) = caps.formats.iter().copied().find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
            )
        }) {
            config.format = best;
        }

        config.alpha_mode = if transparent {
            gfx.pick_alpha_mode(&surface)
        } else {
            wgpu::CompositeAlphaMode::Opaque
        };
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // 遮罩是"呼出即用"的交互，帧延迟压到 1 让拖拽跟手
        config.desired_maximum_frame_latency = 1;
        surface.configure(&gfx.device, &config);

        let egui = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gfx.device.limits().max_texture_dimension_2d as usize),
        );

        let painter = Renderer::new(&gfx.device, config.format, RendererOptions::default());

        Ok(Self {
            window,
            surface,
            config,
            egui,
            painter,
            viewport,
            clear: if transparent {
                wgpu::Color::TRANSPARENT
            } else {
                wgpu::Color::BLACK
            },
        })
    }

    /// 处理尺寸变化。零尺寸（窗口被最小化）时直接忽略 —— 此时配置 0×0 的
    /// 交换链会让 wgpu 报错。
    pub fn resize(&mut self, gfx: &GpuContext, size: winit::dpi::PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.config.width == size.width && self.config.height == size.height {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&gfx.device, &self.config);
    }

    pub fn size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.window.inner_size()
    }

    /// 该窗口内可绘制区域的逻辑尺寸（点）。
    ///
    /// 直接用 `viewport.scale`（= 该屏的 pixels_per_point）而不是 winit 的
    /// `scale_factor()`：两者在绝大多数情况下相等，但一旦窗口被拖到另一块
    /// 缩放的屏幕上、而 `Resized` 事件还没到达时，`viewport` 才是我们真正
    /// 用来换算坐标的那一个。这里跟渲染路径保持一致，避免"工具栏比内容早一帧
    /// 换尺寸"造成的抖动。
    #[allow(dead_code)] // 多显示器版本里工具栏定位会用到
    pub fn size_points(&self) -> egui::Vec2 {
        let s = self.size();
        let scale = self.viewport.scale;
        egui::vec2(s.width as f32 / scale, s.height as f32 / scale)
    }
}
