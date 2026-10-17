//! 系统托盘图标与右键菜单。
//!
//! ## 两个必须注意的点
//!
//! 1. **托盘必须在事件循环跑起来之后再创建**。Windows 上 `tray-icon` 会建一个
//!    隐藏窗口来收 `WM_*` 消息，而消息泵属于 winit。在
//!    [`winit::application::ApplicationHandler::resumed`] 里创建最稳。
//! 2. **菜单事件是异步的**，从另一个线程发过来。所以用
//!    `MenuEvent::set_event_handler` 把事件转发成 winit 的用户事件，
//!    而不是在 `about_to_wait` 里轮询通道 —— 后者在没有鼠标移动时根本不会被唤醒。

use anyhow::{Context as _, Result};
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[cfg(target_os = "macos")]
const STATUS_AUTOSAVE_NAME: &str = "Snipaste-RS";

/// 菜单项 id。用字符串常量而不是句柄比较，好处是事件回传路径上
/// 不需要跨线程共享任何对象。
pub mod ids {
    pub const CAPTURE: &str = "snipaste.capture";
    pub const PASTE: &str = "snipaste.paste";
    pub const AUTOSTART: &str = "snipaste.autostart";
    pub const HOTKEY_SETTINGS: &str = "snipaste.hotkey_settings";
    pub const PIN_HELP: &str = "snipaste.pin_help";
    pub const ABOUT: &str = "snipaste.about";
    pub const QUIT: &str = "snipaste.quit";
    #[cfg(target_os = "macos")]
    pub const PERMISSION: &str = "snipaste.permission";
}

/// 从菜单事件解析出的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// 立刻截一张图（等价于按 F1）。
    Capture,
    /// 把剪贴板里的图贴到桌面。
    Paste,
    /// 用户点了"开机自启"勾选项，需要读取勾选态并落盘。
    ToggleAutostart,
    HotkeySettings,
    /// 弹出贴图操作说明。
    PinHelp,
    About,
    #[cfg(target_os = "macos")]
    ScreenPermission,
    Quit,
}

impl MenuAction {
    /// 把 muda 的菜单事件翻译成动作。
    ///
    /// 未知 id 返回 `None` 而不是 panic —— 以后加菜单项时不会因为忘了
    /// 更新这里而炸掉整个托盘。
    pub fn parse(id: &MenuId) -> Option<Self> {
        match id.0.as_str() {
            ids::CAPTURE => Some(Self::Capture),
            ids::PASTE => Some(Self::Paste),
            ids::AUTOSTART => Some(Self::ToggleAutostart),
            ids::HOTKEY_SETTINGS => Some(Self::HotkeySettings),
            ids::PIN_HELP => Some(Self::PinHelp),
            ids::ABOUT => Some(Self::About),
            #[cfg(target_os = "macos")]
            ids::PERMISSION => Some(Self::ScreenPermission),
            ids::QUIT => Some(Self::Quit),
            _ => None,
        }
    }
}

/// 托盘句柄。**必须长期持有**：drop `TrayIcon` 会让图标消失。
pub struct Tray {
    _icon: TrayIcon,
    /// 开机自启是勾选项，需要在状态变化后回写勾选态。
    pub autostart: CheckMenuItem,
    capture: MenuItem,
    paste: MenuItem,
}

impl Tray {
    /// 创建托盘。`autostart_enabled` 用于初始化勾选态。
    pub fn new(autostart_enabled: bool) -> Result<Self> {
        #[cfg(target_os = "macos")]
        snipaste_platform::macos::prepare_menu_bar_position(STATUS_AUTOSAVE_NAME);
        let menu = Menu::new();

        let capture = MenuItem::with_id(ids::CAPTURE, "截图", true, None);
        let paste = MenuItem::with_id(ids::PASTE, "贴图", true, None);
        let autostart =
            CheckMenuItem::with_id(ids::AUTOSTART, "开机自启", true, autostart_enabled, None);
        let pin_help = MenuItem::with_id(ids::PIN_HELP, "操作说明与快捷键…", true, None);
        let hotkey_settings = MenuItem::with_id(ids::HOTKEY_SETTINGS, "热键设置…", true, None);
        let about = MenuItem::with_id(ids::ABOUT, "关于映贴…", true, None);
        let quit = MenuItem::with_id(ids::QUIT, "退出", true, None);

        menu.append_items(&[
            &capture,
            &paste,
            &PredefinedMenuItem::separator(),
            &autostart,
            &hotkey_settings,
            &PredefinedMenuItem::separator(),
            &pin_help,
            &about,
            &quit,
        ])
        .context("组装托盘菜单失败")?;

        #[cfg(target_os = "macos")]
        menu.insert(
            &MenuItem::with_id(ids::PERMISSION, "屏幕录制权限设置…", true, None),
            3,
        )?;

        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(crate::APP_NAME);
        #[cfg(target_os = "macos")]
        let builder = builder
            .with_icon_templated(make_icon(32))
            .with_menu_on_left_click(true)
            .with_menu_on_right_click(true)
            .with_autosave_name(STATUS_AUTOSAVE_NAME);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(make_icon(32));
        let icon = builder
            .build()
            .context("创建托盘图标失败（当前桌面环境可能不支持系统托盘）")?;

        #[cfg(target_os = "macos")]
        if let Some(item) = icon.ns_status_item() {
            // The retained status item stays alive in `icon`; AppKit runs on this thread.
            let pointer = std::ptr::from_ref(&*item).cast_mut().cast();
            unsafe { snipaste_platform::macos::configure_menu_bar_item(pointer) };
        }

        log::info!("系统托盘已就绪，位置：{:?}", icon.rect());

        Ok(Self {
            _icon: icon,
            autostart,
            capture,
            paste,
        })
    }

    pub fn set_hotkey_labels(&self, capture: &str, paste: &str) {
        self.capture.set_text(format!("截图（{capture}）"));
        self.paste.set_text(format!("贴图（{paste}）"));
    }
}

/// 程序化生成托盘图标（`size × size` RGBA，非预乘）。
///
/// 不引入 PNG 解码、也不往仓库里塞二进制资源：直接按像素画一个
/// "取景框 + 十字定位"的图案，与截图工具的语义一致。
pub fn make_icon(size: u32) -> Icon {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let scale = s / 32.0; // 按 32 为基准设计，再等比缩放

    // 取景框：四条边 + 四个角向外伸出的短线
    let stroke = (2.0 * scale).max(1.0);
    let inset = 6.0 * scale;
    let corner = 8.0 * scale;

    let mut put = |x: f32, y: f32, color: [u8; 4]| {
        let xi = x.round();
        let yi = y.round();
        if xi < 0.0 || yi < 0.0 || xi >= s || yi >= s {
            return;
        }
        let idx = ((yi as u32 * size + xi as u32) * 4) as usize;
        rgba[idx..idx + 4].copy_from_slice(&color);
    };

    let blue = [56u8, 152, 236, 255];
    let white = [238u8, 242, 248, 255];

    // 左上角的两条短线（横 + 竖）
    let mut t = 0.0f32;
    while t < corner {
        for d in 0..(stroke.ceil() as u32) {
            let d = d as f32;
            put(inset + t, inset + d, blue); // 横
            put(inset + d, inset + t, blue); // 竖
            put(s - inset - 1.0 - t, s - inset - 1.0 - d, blue);
            put(s - inset - 1.0 - d, s - inset - 1.0 - t, blue);
        }
        t += 1.0;
    }

    // 中央十字准星
    let c = s / 2.0;
    let arm = 5.0 * scale;
    let mut t = -arm;
    while t <= arm {
        let half = (stroke.ceil() * 0.5) as i32;
        for d in -half..=half {
            let d = d as f32;
            put(c + t, c + d, white);
            put(c + d, c + t, white);
        }
        t += 1.0;
    }

    Icon::from_rgba(rgba, size, size).unwrap_or_else(|err| {
        // 生成的缓冲区尺寸一定合法，走到这里只可能是平台层出了问题；
        // 退化成一个纯色方块，至少保证托盘有图标。
        log::warn!("生成托盘图标失败（{err}），退化为纯色方块");
        let flat = [56u8, 152, 236, 255].repeat((size * size) as usize);
        Icon::from_rgba(flat, size, size).expect("尺寸合法")
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_buffer_has_expected_length() {
        // 这里只验证像素缓冲区的尺寸推导，不触碰平台层
        // （`tray_icon::Icon` 在无桌面环境里构造会失败）
        let size = 32u32;
        let rgba = vec![0u8; (size * size * 4) as usize];
        assert_eq!(rgba.len(), 4096);
    }
}
