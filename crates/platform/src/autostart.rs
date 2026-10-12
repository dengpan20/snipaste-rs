//! 仅在用户点击托盘选项时改变当前用户的开机启动项。
pub fn enabled() -> bool {
    imp::enabled()
}
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    imp::set_enabled(enabled)
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    const KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE: &str = "Snipaste-RS";
    fn command() -> std::process::Command {
        let mut command = std::process::Command::new("reg.exe");
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        command
    }
    pub fn enabled() -> bool {
        command()
            .args(["query", KEY, "/v", VALUE])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    pub fn set_enabled(wanted: bool) -> Result<(), String> {
        let mut command = command();
        if wanted {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let value = format!("\"{}\"", exe.display());
            command.args(["add", KEY, "/v", VALUE, "/t", "REG_SZ", "/d", &value, "/f"]);
        } else {
            if !enabled() {
                return Ok(());
            }
            command.args(["delete", KEY, "/v", VALUE, "/f"]);
        }
        let output = command.output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "无法更新用户启动项：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use crate::launch_agent::{self, LABEL};
    use std::path::PathBuf;

    fn agent_path() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("找不到用户主目录")?;
        let home = PathBuf::from(home);
        if !home.is_absolute() {
            return Err("用户主目录不是绝对路径".into());
        }
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }
    pub fn enabled() -> bool {
        agent_path().is_ok_and(|path| path.is_file())
    }
    pub fn set_enabled(wanted: bool) -> Result<(), String> {
        let path = agent_path()?;
        if wanted {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let bundle = exe
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"))
                .ok_or("开机自启需要从映贴.app 启动")?;
            // LaunchServices opens the .app, preserving its screen-recording identity.
            let content = launch_agent::plist(bundle)?;
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let temporary = path.with_extension("plist.tmp");
            std::fs::write(&temporary, content).map_err(|e| e.to_string())?;
            std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
        } else if path.exists() {
            // Ignore an unloaded job (common when enabled during this session).
            // No KeepAlive job and no operation on the running application.
            extern "C" {
                fn getuid() -> u32;
            }
            let service = format!("gui/{}/{LABEL}", unsafe { getuid() });
            let _ = std::process::Command::new("/bin/launchctl")
                .args(["bootout", &service])
                .output();
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn enabled() -> bool {
        false
    }
    pub fn set_enabled(_: bool) -> Result<(), String> {
        Err("当前平台尚未提供开机启动设置".into())
    }
}
