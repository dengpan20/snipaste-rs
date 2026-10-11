//! 全局热键注册。
//!
//! `global-hotkey` 在 Windows 上走 `RegisterHotKey`，在 macOS 上走
//! Carbon `RegisterEventHotKey`；两者都是**系统级独占**注册，所以：
//!
//! * 热键被别的软件占用时会注册失败。这在本项目里是**必然会遇到**的情况 ——
//!   用户的机器上很可能就装着正版 Snipaste，而它的默认截图键同样是 F1。
//!   因此这里准备了一串候选键，依次尝试，并把最终生效的那个暴露出去给界面显示。
//! * `GlobalHotKeyManager` 一旦被 drop，注册立即失效。所以它必须被长期持有
//!   （`Hotkeys` 里 `_manager` 字段存在的唯一理由就是这个）。

use anyhow::{Context as _, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

/// 全局热键能触发的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 呼出 / 收起截图遮罩。
    Capture,
    /// 把上一张截图贴到桌面。
    PasteLast,
}

/// 截图热键的候选列表，按优先级排列。
///
/// 第一项是无修饰键的 `F1` —— Snipaste 的默认键，照搬以降低迁移成本。
/// 后面几项是**降级方案**：当 F1 被占用时至少还能用，而不是功能直接失效。
#[cfg(not(target_os = "macos"))]
const CAPTURE_CANDIDATES: &[(&str, Option<Modifiers>, Code)] = &[
    ("F1", None, Code::F1),
    (
        "Ctrl+Shift+F1",
        Some(Modifiers::CONTROL.union(Modifiers::SHIFT)),
        Code::F1,
    ),
    (
        "Ctrl+Alt+F1",
        Some(Modifiers::CONTROL.union(Modifiers::ALT)),
        Code::F1,
    ),
    ("PrintScreen", None, Code::PrintScreen),
];

#[cfg(target_os = "macos")]
const CAPTURE_CANDIDATES: &[(&str, Option<Modifiers>, Code)] = &[
    (
        "⌘⇧2",
        Some(Modifiers::SUPER.union(Modifiers::SHIFT)),
        Code::Digit2,
    ),
    ("F1", None, Code::F1),
];

/// 已注册的热键集合。
pub struct Hotkeys {
    /// 唯一目的是持有管理器：drop 它等于注销所有热键。
    _manager: GlobalHotKeyManager,
    capture: Option<HotKey>,
    /// 实际生效的截图热键名，给界面显示用。
    pub capture_label: String,
    paste: Option<HotKey>,
    capture_alias: Option<HotKey>,
    paste_alias: Option<HotKey>,
    pub paste_label: String,
}

impl Hotkeys {
    /// 注册热键。
    ///
    /// 只有"连热键管理器都建不起来"才算致命错误；某个具体按键被占用不是错误，
    /// 我们会降级到下一位候选，并把这个事实显著地告诉用户。
    pub fn register() -> Result<Self> {
        let manager = GlobalHotKeyManager::new().context("初始化全局热键管理器失败")?;

        let mut capture = None;
        let mut capture_label = String::from("未注册（所有候选键都被占用）");
        let mut failures: Vec<String> = Vec::new();

        for (label, mods, code) in CAPTURE_CANDIDATES {
            match manager.register(HotKey::new(*mods, *code)) {
                Ok(()) => {
                    if failures.is_empty() {
                        log::info!("已注册全局热键 {label} → 截图");
                    } else {
                        log::warn!(
                            "{} 的候选截图热键都已被占用（{}），\
                             现已降级为 {label} → 截图。\
                             想用 F1 请先退出占用它的程序（例如正版 Snipaste）。",
                            failures.join("、"),
                            failures.len()
                        );
                    }
                    capture = Some(HotKey::new(*mods, *code));
                    capture_label = (*label).to_owned();
                    break;
                }
                Err(err) => failures.push(format!("{label}: {err}")),
            }
        }

        if capture.is_none() {
            log::error!(
                "无法注册任何截图热键：{}。托盘菜单与界面按钮仍可触发截图。",
                failures.join("；")
            );
        }

        // 贴图热键失败完全不影响主流程
        let (paste_label, paste) = if cfg!(target_os = "macos") {
            (
                "⌘⇧1",
                HotKey::new(Some(Modifiers::SUPER.union(Modifiers::SHIFT)), Code::Digit1),
            )
        } else {
            ("F3", HotKey::new(None, Code::F3))
        };
        let paste = match manager.register(paste) {
            Ok(()) => Some(paste),
            Err(err) => {
                log::warn!("注册 {paste_label}（贴图）失败：{err}；托盘菜单仍可用");
                None
            }
        };
        let mut paste_label = if paste.is_some() {
            paste_label.to_owned()
        } else {
            "未注册".into()
        };
        let (capture_alias, paste_alias) = if cfg!(target_os = "macos") {
            let alias = HotKey::new(None, Code::F1);
            let capture_alias = if capture != Some(alias) && manager.register(alias).is_ok() {
                capture_label.push_str(" / F1");
                Some(alias)
            } else {
                None
            };
            let alias = HotKey::new(None, Code::F3);
            let paste_alias = if manager.register(alias).is_ok() {
                if paste.is_some() {
                    paste_label.push_str(" / F3");
                } else {
                    paste_label = "F3".into();
                }
                Some(alias)
            } else {
                None
            };
            (capture_alias, paste_alias)
        } else {
            (None, None)
        };

        Ok(Self {
            _manager: manager,
            capture,
            capture_label,
            paste,
            capture_alias,
            paste_alias,
            paste_label,
        })
    }

    /// 把热键 id 翻译成动作。id 由 `global-hotkey` 按「修饰键位 + 按键码」算出。
    pub fn action_for(&self, id: u32) -> Option<Action> {
        if self.capture.map(|h| h.id()) == Some(id)
            || self.capture_alias.map(|h| h.id()) == Some(id)
        {
            Some(Action::Capture)
        } else if self.paste.map(|h| h.id()) == Some(id)
            || self.paste_alias.map(|h| h.id()) == Some(id)
        {
            Some(Action::PasteLast)
        } else {
            None
        }
    }
}
