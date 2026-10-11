use std::path::PathBuf;
/// 返回 None 表示用户取消；错误与取消分开反馈。
pub fn save_image_path() -> Result<Option<PathBuf>, String> {
    imp::save_image_path()
}

pub fn show_information(title: &str, message: &str) {
    imp::show_information(title, message);
}
#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    pub fn show_information(title: &str, message: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
    pub fn save_image_path() -> Result<Option<PathBuf>, String> {
        let mut file = [0u16; 32768];
        let name: Vec<u16> = "截图.png".encode_utf16().collect();
        file[..name.len()].copy_from_slice(&name);
        let filter: Vec<u16> = "PNG 图片\0*.png\0JPEG 图片\0*.jpg;*.jpeg\0\0"
            .encode_utf16()
            .collect();
        let ext: Vec<u16> = "png\0".encode_utf16().collect();
        let title: Vec<u16> = "保存截图\0".encode_utf16().collect();
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog.lpstrFile = file.as_mut_ptr();
        dialog.nMaxFile = file.len() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.nFilterIndex = 1;
        dialog.lpstrDefExt = ext.as_ptr();
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if unsafe { GetSaveFileNameW(&mut dialog) } != 0 {
            let len = file.iter().position(|v| *v == 0).unwrap_or(file.len());
            Ok(Some(PathBuf::from(String::from_utf16_lossy(&file[..len]))))
        } else {
            let error = unsafe { CommDlgExtendedError() };
            if error == 0 {
                Ok(None)
            } else {
                Err(format!("保存对话框错误：{error}"))
            }
        }
    }
}
#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    pub fn save_image_path() -> Result<Option<PathBuf>, String> {
        Ok(crate::macos::save_image_path())
    }
    pub use crate::macos::show_information;
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;
    pub fn save_image_path() -> Result<Option<PathBuf>, String> {
        Err("当前平台尚未实现保存对话框".into())
    }
    pub fn show_information(title: &str, message: &str) {
        log::info!("{title}: {message}");
    }
}
