//! # 映贴
//!
//! 跨端截图标注工具（Snipaste 复刻）。Rust 原生实现，macOS 平台能力用薄 Objective-C 桥：
//! `winit` 管窗口、`egui` 管界面、`wgpu` 管渲染，没有 WebView、没有 JS 运行时。
//!
//! 各层职责见 `docs/architecture.md`：
//!
//! ```text
//! snipaste-core      领域模型（零平台依赖，纯逻辑，可单测）
//! snipaste-render    领域模型 → egui 图元
//! snipaste-capture   屏幕捕获（Windows: GDI；macOS 14+: ScreenCaptureKit）
//! snipaste-platform  热键、自启、剪贴板、权限
//! snipaste-app       事件循环、窗口与状态机   ← 本 crate
//! ```

// 发行版不要弹出黑框控制台；debug 构建保留，方便看日志
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("macOS builds support Apple Silicon only; use aarch64-apple-darwin");

#[cfg(debug_assertions)]
mod acceptance;
mod app;
mod color_picker;
mod editor;
mod export;
mod gfx;
mod hotkey;
mod memory_profile;
mod pin;
mod qr;
mod save;
#[cfg(any(target_os = "macos", test))]
mod screen_permission;
mod settings;
mod tray;
mod ui;

use anyhow::{Context as _, Result};
use winit::event_loop::{ControlFlow, EventLoop};

/// User-facing name; executable and bundle identifiers stay stable across renames.
pub const APP_NAME: &str = "映贴";

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    log::info!("{APP_NAME} {} 启动", env!("CARGO_PKG_VERSION"));

    let mut builder = EventLoop::<app::UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_default_menu(false)
            .with_activate_ignoring_other_apps(false);
    }
    let event_loop = builder.build().context("创建 winit 事件循环失败")?;

    // Wait 模式：没有事件时线程真正睡着。
    // 截图工具绝大部分时间在后台待命，轮询式的事件循环会白白耗电 ——
    // 跨线程事件（热键、托盘菜单）通过 EventLoopProxy 主动唤醒，不会漏。
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut application = app::App::new(event_loop.create_proxy());
    event_loop
        .run_app(&mut application)
        .context("事件循环异常退出")?;

    log::info!("{APP_NAME} 已退出");
    Ok(())
}
