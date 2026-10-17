//! 可持久化的全局热键。默认键保留候选降级，自定义键冲突时明确报错。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Capture,
    PasteLast,
}

impl Action {
    pub const ALL: [Self; 2] = [Self::Capture, Self::PasteLast];

    pub fn label(self) -> &'static str {
        match self {
            Self::Capture => "截图",
            Self::PasteLast => "桌面贴图",
        }
    }

    pub fn default_key(self) -> HotKey {
        match self {
            Self::Capture => capture_candidates()[0],
            Self::PasteLast if cfg!(target_os = "macos") => {
                HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Digit1)
            }
            Self::PasteLast => HotKey::new(None, Code::F3),
        }
    }
}

/// None 表示平台默认策略，包括候选热键和 macOS 的 F1/F3 别名。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub capture: Option<String>,
    pub paste: Option<String>,
}

impl Config {
    pub fn binding(&self, action: Action) -> &Option<String> {
        match action {
            Action::Capture => &self.capture,
            Action::PasteLast => &self.paste,
        }
    }

    pub fn set(&mut self, action: Action, key: HotKey) {
        *match action {
            Action::Capture => &mut self.capture,
            Action::PasteLast => &mut self.paste,
        } = Some(key.to_string());
    }

    pub fn key(&self, action: Action) -> Result<HotKey> {
        match self.binding(action) {
            Some(value) => parse(value),
            None => Ok(action.default_key()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let capture = self.key(Action::Capture).context("截图热键无效")?;
        let paste = self.key(Action::PasteLast).context("贴图热键无效")?;
        if capture == paste {
            bail!("截图和贴图不能使用同一热键，请修改其中一个。");
        }
        Ok(())
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&config_path()?)
    }

    fn load_from(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).context("读取热键设置失败"),
        };
        let config: Self = serde_json::from_slice(&bytes).context("热键设置文件格式无效")?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path()?)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().context("热键设置路径无效")?;
        std::fs::create_dir_all(parent).context("创建设置目录失败")?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            use std::io::Write as _;
            let mut file = std::fs::File::create(&temporary).context("创建临时设置文件失败")?;
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path).context("保存热键设置失败")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

fn config_path() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let base = base
        .filter(|path| path.is_absolute())
        .context("找不到用户设置目录")?;
    Ok(base.join("Snipaste-RS").join("hotkeys.json"))
}

pub fn parse(value: &str) -> Result<HotKey> {
    let key = value.parse::<HotKey>().context("无法识别这个按键组合")?;
    let function_key = matches!(
        key.key,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
            | Code::F16
            | Code::F17
            | Code::F18
            | Code::F19
            | Code::F20
            | Code::F21
            | Code::F22
            | Code::F23
            | Code::F24
            | Code::PrintScreen
    );
    if !function_key
        && !key
            .mods
            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        bail!(
            "普通按键需要搭配 Ctrl、Alt 或 {}，也可以单独使用 F1–F24 / PrintScreen。",
            if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Win"
            }
        );
    }
    if key.key == Code::Escape {
        bail!("Esc 用于取消操作，请选择其他热键。");
    }
    Ok(key)
}

pub fn label(key: HotKey) -> String {
    let mut parts = Vec::new();
    for (modifier, name) in [
        (Modifiers::CONTROL, "Ctrl"),
        (
            Modifiers::ALT,
            if cfg!(target_os = "macos") {
                "Option"
            } else {
                "Alt"
            },
        ),
        (Modifiers::SHIFT, "Shift"),
        (
            Modifiers::SUPER,
            if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Win"
            },
        ),
    ] {
        if key.mods.contains(modifier) {
            parts.push(name.to_owned());
        }
    }
    let code = key.key.to_string();
    parts.push(
        code.strip_prefix("Key")
            .or_else(|| code.strip_prefix("Digit"))
            .unwrap_or(&code)
            .to_owned(),
    );
    parts.join("+")
}

fn capture_candidates() -> Vec<HotKey> {
    if cfg!(target_os = "macos") {
        vec![
            HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Digit2),
            HotKey::new(None, Code::F1),
        ]
    } else {
        vec![
            HotKey::new(None, Code::F1),
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::F1),
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::F1),
            HotKey::new(None, Code::PrintScreen),
        ]
    }
}

#[derive(Default)]
struct Registered {
    bindings: Vec<(HotKey, Action)>,
    labels: Vec<String>,
    failures: Vec<(Action, String)>,
}

impl Registered {
    fn require_custom_active(&self, config: &Config) -> Result<()> {
        let errors: Vec<_> = self
            .failures
            .iter()
            .filter(|(action, _)| config.binding(*action).is_some())
            .map(|(_, error)| error.as_str())
            .collect();
        if !errors.is_empty() {
            bail!("{}", errors.join("\n"));
        }
        Ok(())
    }
}

/// 注册算法与系统调用分离，测试可模拟外部占用。
fn register_bindings(
    config: &Config,
    mut register: impl FnMut(HotKey) -> Result<()>,
) -> Result<Registered> {
    config.validate()?;
    let mut state = Registered::default();
    for action in Action::ALL {
        let requested = config.key(action)?;
        let candidates = if action == Action::Capture && config.binding(action).is_none() {
            capture_candidates()
        } else {
            vec![requested]
        };
        let other = if action == Action::Capture {
            Action::PasteLast
        } else {
            Action::Capture
        };
        let mut active = Vec::new();
        let mut failures = Vec::new();
        for key in candidates {
            // 候选键和别名不能抢占另一个功能明确选择的热键。
            if config.binding(other).is_some() && key == config.key(other)? {
                continue;
            }
            match register(key) {
                Ok(()) => {
                    state.bindings.push((key, action));
                    active.push(label(key));
                    break;
                }
                Err(err) => failures.push(format!("{}：{err}", label(key))),
            }
        }
        if cfg!(target_os = "macos") && config.binding(action).is_none() {
            let alias = HotKey::new(
                None,
                if action == Action::Capture {
                    Code::F1
                } else {
                    Code::F3
                },
            );
            if !state.bindings.iter().any(|(key, _)| *key == alias)
                && alias != config.key(other)?
                && register(alias).is_ok()
            {
                state.bindings.push((alias, action));
                active.push(label(alias));
            }
        }
        if active.is_empty() {
            let error = format!(
                "{}热键 {} 无法注册，可能已被其他应用或系统占用。请选择其他组合。",
                action.label(),
                label(requested),
            );
            log::warn!("{error} {}", failures.join("；"));
            state.failures.push((action, error));
            state.labels.push(format!("{} 未注册", label(requested)));
        } else {
            state.labels.push(active.join(" / "));
        }
    }
    Ok(state)
}

pub struct Hotkeys {
    // 管理器销毁时注销全部按键；设置窗口打开时释放，以便录入原有按键。
    _manager: GlobalHotKeyManager,
    state: Registered,
    pub capture_label: String,
    pub paste_label: String,
}

impl Hotkeys {
    pub fn register(config: &Config) -> Result<Self> {
        let manager = GlobalHotKeyManager::new().context("初始化全局热键管理器失败")?;
        let state = register_bindings(config, |key| manager.register(key).map_err(Into::into))?;
        log::info!(
            "全局热键：截图 {}；贴图 {}",
            state.labels[0],
            state.labels[1]
        );
        Ok(Self {
            _manager: manager,
            capture_label: state.labels[0].clone(),
            paste_label: state.labels[1].clone(),
            state,
        })
    }

    pub fn require_custom_active(&self, config: &Config) -> Result<()> {
        self.state.require_custom_active(config)
    }

    pub fn action_for(&self, id: u32) -> Option<Action> {
        self.state
            .bindings
            .iter()
            .find(|(key, _)| key.id() == id)
            .map(|(_, action)| *action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_duplicates_and_typing_keys() {
        let config = Config {
            capture: Some("ctrl+shift+A".into()),
            paste: Some("Shift+Control+KeyA".into()),
        };
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("同一热键"));
        assert!(parse("A").is_err());
        assert!(parse("Shift+A").is_err());
        assert!(parse("Ctrl+Escape").is_err());
        assert!(parse("F12").is_ok());
        assert!(parse("Alt+Digit2").is_ok());
    }

    #[test]
    fn custom_conflict_does_not_fall_back() {
        let config = Config {
            capture: Some("Ctrl+Alt+A".into()),
            paste: Some("Ctrl+Alt+B".into()),
        };
        let state = register_bindings(&config, |key| {
            if key == config.key(Action::Capture)? {
                bail!("occupied")
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(state.failures.len(), 1);
        assert!(state.require_custom_active(&config).is_err());
        assert_eq!(state.bindings.len(), 1);
        assert_eq!(state.bindings[0].1, Action::PasteLast);
    }

    #[test]
    fn default_fallback_reserves_custom_bindings() {
        let candidates = capture_candidates();
        let reserved = candidates[1];
        let config = Config {
            capture: None,
            paste: Some(reserved.to_string()),
        };
        let mut attempts = Vec::new();
        let state = register_bindings(&config, |key| {
            attempts.push(key);
            if key == candidates[0] {
                bail!("occupied")
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(attempts.iter().filter(|key| **key == reserved).count(), 1);
        assert!(state.bindings.contains(&(reserved, Action::PasteLast)));
        assert!(!state.bindings.contains(&(reserved, Action::Capture)));
    }

    #[test]
    fn unavailable_defaults_can_be_restored() {
        let config = Config::default();
        let state = register_bindings(&config, |_| bail!("occupied")).unwrap();
        assert_eq!(state.failures.len(), 2);
        assert!(state.require_custom_active(&config).is_ok());
    }

    #[test]
    fn settings_round_trip_and_replacement() {
        let dir = std::env::temp_dir().join(format!("snipaste-hotkeys-{}", std::process::id()));
        let path = dir.join("hotkeys.json");
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
        let config = Config {
            capture: Some("Ctrl+Alt+A".into()),
            paste: Some("Ctrl+Alt+B".into()),
        };
        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);
        Config::default().save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
        std::fs::write(&path, "invalid").unwrap();
        assert!(Config::load_from(&path).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
