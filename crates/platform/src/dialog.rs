use std::path::{Path, PathBuf};
/// 返回 None 表示用户取消；错误与取消分开反馈。
pub fn save_image_path() -> Result<Option<PathBuf>, String> {
    save_image_path_with_options(None, false)
}

pub fn save_image_path_with_options(
    directory: Option<&Path>,
    jpeg: bool,
) -> Result<Option<PathBuf>, String> {
    imp::save_image_path(directory, jpeg)
}

pub fn choose_directory() -> Result<Option<PathBuf>, String> {
    imp::choose_directory()
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
    pub fn save_image_path(
        directory: Option<&Path>,
        jpeg: bool,
    ) -> Result<Option<PathBuf>, String> {
        use std::os::windows::ffi::OsStrExt;
        let mut file = [0u16; 32768];
        let name: Vec<u16> = if jpeg { "截图.jpg" } else { "截图.png" }
            .encode_utf16()
            .collect();
        file[..name.len()].copy_from_slice(&name);
        let filter: Vec<u16> = "PNG 图片\0*.png\0JPEG 图片\0*.jpg;*.jpeg\0\0"
            .encode_utf16()
            .collect();
        let ext: Vec<u16> = if jpeg { "jpg\0" } else { "png\0" }
            .encode_utf16()
            .collect();
        let directory = directory.map(|p| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        });
        let title: Vec<u16> = "保存截图\0".encode_utf16().collect();
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog.lpstrFile = file.as_mut_ptr();
        dialog.nMaxFile = file.len() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.nFilterIndex = if jpeg { 2 } else { 1 };
        dialog.lpstrInitialDir = directory.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
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

    pub fn choose_directory() -> Result<Option<PathBuf>, String> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::{System::Com::*, UI::Shell::*};
        let title: Vec<u16> = "选择截图保存目录\0".encode_utf16().collect();
        unsafe {
            let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            let mut info: BROWSEINFOW = std::mem::zeroed();
            info.lpszTitle = title.as_ptr();
            info.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
            let item = SHBrowseForFolderW(&info);
            let result = if item.is_null() {
                Ok(None)
            } else {
                let mut path = [0u16; 32768];
                let ok = SHGetPathFromIDListW(item, path.as_mut_ptr());
                CoTaskMemFree(item.cast());
                if ok == 0 {
                    Err("无法获取所选文件夹路径".into())
                } else {
                    let len = path.iter().position(|v| *v == 0).unwrap_or(path.len());
                    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
                        &path[..len],
                    ))))
                }
            };
            if initialized >= 0 {
                CoUninitialize();
            }
            result
        }
    }
}
#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    pub fn save_image_path(
        directory: Option<&Path>,
        jpeg: bool,
    ) -> Result<Option<PathBuf>, String> {
        Ok(crate::macos::save_image_path_with_options(directory, jpeg))
    }
    pub fn choose_directory() -> Result<Option<PathBuf>, String> {
        Ok(crate::macos::choose_directory())
    }
    pub use crate::macos::show_information;
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;
    pub fn save_image_path(
        _directory: Option<&Path>,
        _jpeg: bool,
    ) -> Result<Option<PathBuf>, String> {
        Err("当前平台尚未实现保存对话框".into())
    }
    pub fn choose_directory() -> Result<Option<PathBuf>, String> {
        Err("当前平台尚未实现文件夹选择".into())
    }
    pub fn show_information(title: &str, message: &str) {
        log::info!("{title}: {message}");
    }
}
