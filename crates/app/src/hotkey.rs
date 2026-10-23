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
        let mut hotkeys = Self {
            _manager: GlobalHotKeyManager::new().context("初始化全局热键管理器失败")?,
            capture: None,
            paste: None,
            capture_alias: None,
            paste_alias: None,
            capture_label: String::new(),
            paste_label: String::new(),
        };
        hotkeys.reconfigure("", "")?;
        Ok(hotkeys)
    }

    /// Register a complete replacement, restoring the previous bindings on error.
    pub fn reconfigure(&mut self, capture_text: &str, paste_text: &str) -> Result<()> {
        self.reconfigure_with(capture_text, paste_text, || Ok(()))
    }

    pub fn reconfigure_with(
        &mut self,
        capture_text: &str,
        paste_text: &str,
        commit: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        validate_pair(capture_text, paste_text)?;
        let custom_capture = parse_optional(capture_text)?;
        let custom_paste = parse_optional(paste_text)?;
        let old: Vec<_> = [
            self.capture,
            self.paste,
            self.capture_alias,
            self.paste_alias,
        ]
        .into_iter()
        .flatten()
        .collect();
        self._manager
            .unregister_all(&old)
            .context("无法注销旧快捷键")?;
        let mut registered = Vec::new();
        let result = (|| -> Result<_> {
            let candidates: Vec<_> = if let Some(key) = custom_capture {
                vec![(capture_text.trim().to_owned(), key)]
            } else {
                CAPTURE_CANDIDATES
                    .iter()
                    .map(|(label, mods, key)| ((*label).to_owned(), HotKey::new(*mods, *key)))
                    .collect()
            };
            let mut capture = None;
            let mut capture_label = "未注册，请用菜单截图".to_owned();
            for (label, key) in candidates {
                match self._manager.register(key) {
                    Ok(()) => {
                        registered.push(key);
                        capture = Some(key);
                        capture_label = label;
                        break;
                    }
                    Err(err) if custom_capture.is_some() => {
                        anyhow::bail!("截图快捷键被占用或无法注册：{err}")
                    }
                    Err(err) => log::warn!("无法注册截图快捷键 {label}：{err}"),
                }
            }
            let (default_paste_label, default_paste) = default_paste();
            let paste_key = custom_paste.unwrap_or(default_paste);
            let mut paste_label = if custom_paste.is_some() {
                paste_text.trim().to_owned()
            } else {
                default_paste_label.into()
            };
            let paste = match self._manager.register(paste_key) {
                Ok(()) => {
                    registered.push(paste_key);
                    Some(paste_key)
                }
                Err(err) if custom_paste.is_some() => {
                    anyhow::bail!("贴图快捷键被占用或无法注册：{err}")
                }
                Err(err) => {
                    log::warn!("无法注册贴图快捷键：{err}");
                    paste_label = "未注册，请用菜单贴图".into();
                    None
                }
            };
            let mut alias = |key: HotKey| {
                if !registered.contains(&key) && self._manager.register(key).is_ok() {
                    registered.push(key);
                    Some(key)
                } else {
                    None
                }
            };
            let capture_alias = if cfg!(target_os = "macos") && custom_capture.is_none() {
                alias(HotKey::new(None, Code::F1))
            } else {
                None
            };
            let paste_alias = if cfg!(target_os = "macos") && custom_paste.is_none() {
                alias(HotKey::new(None, Code::F3))
            } else {
                None
            };
            if capture_alias.is_some() {
                capture_label.push_str(" / F1");
            }
            if paste_alias.is_some() {
                paste_label.push_str(" / F3");
            }
            commit()?;
            Ok((
                capture,
                paste,
                capture_alias,
                paste_alias,
                capture_label,
                paste_label,
            ))
        })();
        match result {
            Ok((capture, paste, capture_alias, paste_alias, capture_label, paste_label)) => {
                self.capture = capture;
                self.paste = paste;
                self.capture_alias = capture_alias;
                self.paste_alias = paste_alias;
                self.capture_label = capture_label;
                self.paste_label = paste_label;
                Ok(())
            }
            Err(err) => {
                let _ = self._manager.unregister_all(&registered);
                for key in old {
                    self._manager.register(key).context("恢复原快捷键失败")?;
                }
                Err(err)
            }
        }
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

fn default_paste() -> (&'static str, HotKey) {
    if cfg!(target_os = "macos") {
        (
            "⌘⇧1",
            HotKey::new(Some(Modifiers::SUPER.union(Modifiers::SHIFT)), Code::Digit1),
        )
    } else {
        ("F3", HotKey::new(None, Code::F3))
    }
}

fn parse_optional(text: &str) -> Result<Option<HotKey>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(text.trim().parse::<HotKey>().context(
        "快捷键格式无效，请使用 Ctrl+Shift+S 或 Command+Shift+2 等格式",
    )?))
}

pub fn validate_pair(capture: &str, paste: &str) -> Result<()> {
    let capture = parse_optional(capture)?.unwrap_or_else(|| {
        let (_, mods, key) = CAPTURE_CANDIDATES[0];
        HotKey::new(mods, key)
    });
    let paste = parse_optional(paste)?.unwrap_or_else(|| default_paste().1);
    anyhow::ensure!(capture != paste, "截图与贴图不能使用同一个快捷键");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hotkeys_validate_before_touching_system_registrations() {
        assert!(validate_pair("Ctrl+Shift+F8", "Alt+F9").is_ok());
        assert!(validate_pair("Command+Shift+2", "Command+Shift+1").is_ok());
        assert!(validate_pair("Ctrl+F8", "Control+F8").is_err());
        assert!(validate_pair("Ctrl+Oops", "").is_err());
        assert!(validate_pair("", "").is_ok());
    }
}
