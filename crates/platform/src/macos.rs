//! AppKit/ScreenCaptureKit bridge. Main-thread APIs are called only by the event loop.
use snipaste_core::{Pos, Rect};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::PathBuf,
    sync::Mutex,
};

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct NativeRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Display {
    id: u32,
    bounds: NativeRect,
    pub scale: f64,
}
#[repr(C)]
struct NativeImage {
    width: u32,
    height: u32,
    error: *mut c_char,
    denied: i32,
}
#[repr(C)]
#[derive(Default)]
struct NativeMemory {
    resident: u64,
    footprint: u64,
    peak_resident: u64,
    peak_footprint: u64,
}

extern "C" {
    fn sp_display_for_cursor(out: *mut Display) -> i32;
    fn sp_cursor(x: *mut f64, y: *mut f64) -> i32;
    fn sp_window_at(x: f64, y: f64, out: *mut NativeRect) -> i32;
    fn sp_screen_permission() -> i32;
    fn sp_open_screen_settings();
    fn sp_capture(display: u32, width: u32, height: u32, pixels: *mut u8) -> NativeImage;
    fn sp_free(pointer: *mut c_void);
    fn sp_configure_window(view: *mut c_void, overlay: i32, display: *const Display);
    fn sp_information(title: *const c_char, message: *const c_char);
    fn sp_screen_permission_prompt(title: *const c_char, message: *const c_char);
    fn sp_configure_menu_bar_item(item: *mut c_void);
    fn sp_prepare_menu_bar_position(autosave_name: *const c_char);
    fn sp_save_path() -> *mut c_char;
    fn sp_chinese_font() -> *mut c_char;
    fn sp_memory(out: *mut NativeMemory) -> i32;
    fn sp_begin_capture();
    fn sp_end_capture();
    fn sp_flush_compositor();
    fn sp_release_unused_memory() -> usize;
    fn sp_prepare_window_preview(
        view: *mut c_void,
        pixels: *const u8,
        width: u32,
        height: u32,
        owner: *mut c_void,
        release: unsafe extern "C" fn(*mut c_void, *const c_void, usize),
    ) -> i32;
    fn sp_clear_window_preview(view: *mut c_void);
}

// One display per capture on macOS. Quartz coordinates use top-left logical
// points; the editor uses those points multiplied by this session's scale.
static CAPTURE_DISPLAY: Mutex<Option<Display>> = Mutex::new(None);

pub fn capture_display() -> Option<Display> {
    *CAPTURE_DISPLAY.lock().unwrap()
}

impl Display {
    fn space(&self) -> crate::capture_coordinates::ScreenSpace {
        crate::capture_coordinates::ScreenSpace {
            logical: self.logical_bounds(),
            scale: self.scale as f32,
        }
    }
    pub fn physical_bounds(&self) -> Rect {
        self.space().physical_bounds()
    }
    pub fn logical_bounds(&self) -> Rect {
        Rect::new(
            self.bounds.x as f32,
            self.bounds.y as f32,
            self.bounds.width as f32,
            self.bounds.height as f32,
        )
    }
}

pub fn select_capture_display() -> Option<Display> {
    // Profiling compares repeated sessions on the first measured display even
    // if the desktop pointer moves. Ordinary capture still follows the cursor.
    if std::env::var_os("SNIPASTE_MEMORY_PROFILE_DIR").is_some() {
        if let Some(display) = capture_display() {
            return Some(display);
        }
    }
    let mut display = Display::default();
    // SAFETY: AppKit is initialized and this is called from resumed/user_event.
    if unsafe { sp_display_for_cursor(&mut display) } == 0 {
        return None;
    }
    *CAPTURE_DISPLAY.lock().unwrap() = Some(display);
    Some(display)
}

pub fn cursor_position() -> Option<Pos> {
    let mut x = 0.0;
    let mut y = 0.0;
    // SAFETY: both pointers refer to writable doubles.
    if unsafe { sp_cursor(&mut x, &mut y) } == 0 {
        return None;
    }
    let display = (*CAPTURE_DISPLAY.lock().unwrap())?;
    Some(display.space().to_physical(Pos::new(x as f32, y as f32)))
}

pub fn window_under_point(point: Pos) -> Option<Rect> {
    let display = (*CAPTURE_DISPLAY.lock().unwrap())?;
    let logical = display.space().to_logical(point);
    let mut bounds = NativeRect::default();
    // SAFETY: native code enumerates system windows and writes one rectangle.
    if unsafe { sp_window_at(logical.x as f64, logical.y as f64, &mut bounds) } == 0 {
        return None;
    }
    display.space().clip_window(Rect::new(
        bounds.x as f32,
        bounds.y as f32,
        bounds.width as f32,
        bounds.height as f32,
    ))
}

pub fn screen_permission() -> bool {
    // SAFETY: read-only TCC preflight; never requests access or displays system UI.
    unsafe { sp_screen_permission() != 0 }
}
pub fn open_screen_settings() {
    unsafe { sp_open_screen_settings() }
}

/// # Safety
/// `item` must be a live NSStatusItem owned by the tray, on the AppKit main thread.
pub unsafe fn configure_menu_bar_item(item: *mut c_void) {
    unsafe { sp_configure_menu_bar_item(item) };
}

pub fn prepare_menu_bar_position(autosave_name: &str) {
    if let Ok(name) = CString::new(autosave_name) {
        unsafe { sp_prepare_menu_bar_position(name.as_ptr()) };
    }
}

pub fn begin_capture() {
    unsafe { sp_begin_capture() }
}
pub fn end_capture() {
    unsafe { sp_end_capture() }
}
pub fn flush_compositor() {
    unsafe { sp_flush_compositor() }
}

pub struct CapturedImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
pub enum CaptureFailure {
    PermissionDenied,
    Other(String),
}

pub fn capture(bounds: Rect) -> Result<CapturedImage, CaptureFailure> {
    let display = (*CAPTURE_DISPLAY.lock().unwrap())
        .ok_or_else(|| CaptureFailure::Other("截图显示器尚未选择".into()))?;
    if bounds != display.physical_bounds() {
        return Err(CaptureFailure::Other("截图区域与显示器坐标不一致".into()));
    }
    let width = bounds.width().round() as u32;
    let height = bounds.height().round() as u32;
    // Blocking ScreenCaptureKit waits run off the AppKit thread, so system
    // completion callbacks cannot deadlock waiting for our event loop.
    std::thread::spawn(move || {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| CaptureFailure::Other("截图尺寸过大".into()))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|e| CaptureFailure::Other(e.to_string()))?;
        bytes.resize(len, 0);
        let native = unsafe { sp_capture(display.id, width, height, bytes.as_mut_ptr()) };
        let error = unsafe { take_string(native.error) };
        if native.denied != 0 {
            return Err(CaptureFailure::PermissionDenied);
        }
        if let Some(error) = error {
            return Err(CaptureFailure::Other(error));
        }
        if native.width != width || native.height != height || bytes.is_empty() {
            return Err(CaptureFailure::Other("截图未返回预期尺寸的像素".into()));
        }
        Ok(CapturedImage {
            bytes,
            width: native.width,
            height: native.height,
        })
    })
    .join()
    .map_err(|_| CaptureFailure::Other("屏幕捕获线程异常退出".into()))?
}

/// # Safety
/// `view` must be a live winit AppKit NSView, and the caller must be on the main thread.
pub unsafe fn configure_window(view: *mut c_void, overlay: bool) {
    let display = capture_display();
    let pointer = display
        .as_ref()
        .map_or(std::ptr::null(), |d| d as *const Display);
    unsafe { sp_configure_window(view, i32::from(overlay), pointer) };
}

/// # Safety
/// `view` must be a live winit AppKit NSView on the main event-loop thread.
pub unsafe fn prepare_window_preview(
    view: *mut c_void,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> bool {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|size| size.checked_mul(4));
    if expected != Some(pixels.len()) || pixels.is_empty() {
        return false;
    }
    // The CGImage provider owns the allocation until AppKit releases the preview.
    // Its callback uses Rust's allocator, and also runs on every native error path.
    unsafe extern "C" fn release(owner: *mut c_void, _: *const c_void, _: usize) {
        unsafe { drop(Box::from_raw(owner.cast::<Vec<u8>>())) };
    }
    let pixels = Box::new(pixels);
    let pointer = pixels.as_ptr();
    let owner = Box::into_raw(pixels).cast();
    unsafe { sp_prepare_window_preview(view, pointer, width, height, owner, release) != 0 }
}

/// # Safety
/// `view` must be a live winit AppKit NSView on the main event-loop thread.
pub unsafe fn clear_window_preview(view: *mut c_void) {
    unsafe { sp_clear_window_preview(view) };
}

pub fn show_information(title: &str, message: &str) {
    if let (Ok(title), Ok(message)) = (CString::new(title), CString::new(message)) {
        unsafe { sp_information(title.as_ptr(), message.as_ptr()) };
    }
}

/// Show the screen-recording explanation with a direct Settings action.
/// Called on the main AppKit event-loop thread after capture permission is denied.
pub fn show_screen_permission_prompt(app_name: &str) {
    let title = CString::new("需要屏幕录制权限").unwrap();
    let message = format!(
        "点击「打开权限设置」，在「屏幕与系统音频录制」中允许 {app_name}。\n\
        如果列表中没有该应用，点击 + 添加 /Applications/{app_name}.app。\n\
        授权后从菜单栏退出应用，再重新打开。"
    );
    if let Ok(message) = CString::new(message) {
        // SAFETY: live NUL-terminated strings remain valid for the modal alert.
        unsafe { sp_screen_permission_prompt(title.as_ptr(), message.as_ptr()) };
    }
}

pub fn save_image_path() -> Option<PathBuf> {
    // Preserve non-UTF8 filesystem paths instead of replacing bytes.
    use std::os::unix::ffi::OsStrExt;
    let raw = unsafe { sp_save_path() };
    if raw.is_null() {
        return None;
    }
    let path =
        unsafe { PathBuf::from(std::ffi::OsStr::from_bytes(CStr::from_ptr(raw).to_bytes())) };
    unsafe { sp_free(raw.cast()) };
    Some(path)
}
pub fn chinese_font_path() -> Option<PathBuf> {
    unsafe { take_string(sp_chinese_font()) }.map(PathBuf::from)
}
pub fn memory_usage() -> Option<crate::memory::Usage> {
    let mut native = NativeMemory::default();
    if unsafe { sp_memory(&mut native) } == 0 {
        return None;
    }
    Some(crate::memory::Usage {
        resident: native.resident as usize,
        private: native.footprint as usize,
        peak_resident: native.peak_resident as usize,
        peak_private: native.peak_footprint as usize,
    })
}

/// Return already-freed allocator caches to macOS after the last graphics session ends.
pub fn release_unused_memory() -> usize {
    unsafe { sp_release_unused_memory() }
}

unsafe fn take_string(raw: *mut c_char) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    let string = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { sp_free(raw.cast()) };
    Some(string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn rejected_preview_releases_its_transferred_owner_once() {
        unsafe extern "C" fn release(owner: *mut c_void, _: *const c_void, _: usize) {
            let count = unsafe { Box::from_raw(owner.cast::<Arc<AtomicUsize>>()) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        let count = Arc::new(AtomicUsize::new(0));
        let owner = Box::into_raw(Box::new(count.clone())).cast();
        let pixels = [255u8; 4];
        // Nil NSView rejects without touching AppKit windows; ownership must still return.
        let result = unsafe {
            sp_prepare_window_preview(std::ptr::null_mut(), pixels.as_ptr(), 1, 1, owner, release)
        };
        assert_eq!(result, 0);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
