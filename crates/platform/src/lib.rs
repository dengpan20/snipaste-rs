//! # snipaste-platform
//!
//! 平台适配层。**所有平台相关的代码只能出现在这个 crate 与 `snipaste-capture`
//! 里** —— `snipaste-core` 是完全纯逻辑的，不允许出现 `#[cfg(target_os)]`。
//!
//! 这样做的收益：上层（`app` / `render`）拿到的永远是与平台无关的纯数据
//! （`snipaste_core` 的 `Rect` / `Pos`），不必到处写 `cfg`；而平台差异
//! 集中在少数几个文件里，将来补 macOS / Linux 时改动面很小。
//!
//! ## 当前提供的能力
//!
//! | 能力 | 状态 |
//! |---|---|
//! | 窗口探测（鼠标下的窗口矩形） | Windows / macOS 14+ |
//! | 开机自启、保存对话框、操作说明 | Windows / macOS 14+ |
//! | 图片 / 文本剪贴板 | ✅ arboard |
//! | DPI 感知 | ⏳ 由 winit 代管，此处无需处理 |
//! | 屏幕录制权限检查 | macOS TCC preflight / request |
//!
//! macOS 原生编译和交互验收步骤见 docs/macos.md。

pub mod autostart;
pub mod clipboard;
pub mod dialog;
pub mod memory;
pub mod window;

#[cfg(any(target_os = "macos", test))]
mod launch_agent;

#[cfg(any(target_os = "macos", test))]
mod capture_coordinates;

#[cfg(target_os = "macos")]
pub mod macos;

pub use window::window_under_point;
