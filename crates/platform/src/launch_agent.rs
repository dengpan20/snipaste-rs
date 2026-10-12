//! Pure LaunchAgent serialization, also tested on Windows.
use std::path::Path;

pub const LABEL: &str = "io.snipaste-rs.app";

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn plist(bundle: &Path) -> Result<String, String> {
    let path = bundle.to_str().ok_or("应用路径不是有效的 UTF-8")?;
    if !bundle.is_absolute() || bundle.extension().and_then(|v| v.to_str()) != Some("app") {
        return Err("请将映贴.app 放入应用程序目录后再开启自启".into());
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array>
<string>/usr/bin/open</string><string>-g</string><string>-a</string><string>{}</string>
</array>
<key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Background</string>
</dict></plist>
"#,
        xml(path)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_path_is_a_single_xml_escaped_argument() {
        let root = if cfg!(windows) {
            "C:/Applications"
        } else {
            "/Applications"
        };
        let bundle = format!("{root}/截图 & <tool> 'test'.app");
        let result = plist(Path::new(&bundle)).unwrap();
        assert!(result.contains("截图 &amp; &lt;tool&gt; &apos;test&apos;.app</string>"));
        assert!(!result.contains("KeepAlive"));
        assert!(plist(Path::new("relative.app")).is_err());
        assert!(plist(Path::new(&format!("{root}/snipaste-rs"))).is_err());
    }
}
