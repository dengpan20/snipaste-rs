//! 窗口探测：取"鼠标指针下方那个窗口"的屏幕矩形。
//!
//! ## 为什么这件事必须在 platform 层
//!
//! `WindowFromPoint` / DWM 这些都是 Win32 独有的；`snipaste-core` 里不允许
//! 出现任何平台代码。所以这里只做**一件事**：把原生查询的结果翻译成
//! `snipaste_core::Rect`（VSS 物理像素），返回给上层。上层拿到的就是纯数据。
//!
//! ## 坐标系
//!
//! Win32 的 `GetWindowRect` / `WindowFromPoint` 返回的是**虚拟桌面物理像素**
//! 坐标，与本项目的 VSS 定义完全一致（原点都是虚拟桌面左上角，单位都是物理
//! 像素）。因此这里**不需要任何换算** —— 前提是进程已开 PerMonitorV2 DPI 感知，
//! 否则 Win32 会把坐标折算成"逻辑单位"，多屏下就全错了。
//!
//! winit 在创建事件循环时会调 `SetProcessDpiAwarenessContext` 把感知级别设好，
//! 所以只要我们是在事件循环跑起来之后才调用，就是安全的。

use snipaste_core::{Pos, Rect};

/// 返回 `point`（VSS 物理像素）**下方最上层窗口**的矩形。
///
/// 返回 `None` 的几种情形（调用方应当退回"整屏"）：
/// - 指针落在桌面 / 壁纸上（那里的窗口是 `Progman` / `WorkerW`，不算真窗口）
/// - 指针下方除本程序自己的遮罩外没有别的窗口
/// - 平台未实现（Linux）
///
/// macOS 使用 CGWindowListCopyWindowInfo，按本次截图显示器的缩放转换。
pub fn window_under_point(point: Pos) -> Option<Rect> {
    imp::window_under_point(point)
}

/// 截图遮罩立即切换可见性，禁用 DWM 的淡入淡出动画。
pub fn disable_transitions(native_handle: isize) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED,
        };
        let disabled: i32 = 1;
        // SAFETY: 调用方传入当前存活窗口的 HWND；数据大小与 BOOL 一致。
        let result = unsafe {
            DwmSetWindowAttribute(
                native_handle as _,
                DWMWA_TRANSITIONS_FORCEDISABLED as _,
                (&disabled as *const i32).cast(),
                std::mem::size_of::<i32>() as u32,
            )
        };
        if result < 0 {
            log::debug!("无法关闭截图窗口动画：{result:#x}");
        }
    }
    #[cfg(not(windows))]
    let _ = native_handle;
}

/// 贴图是桌面工具窗口；在显示前设置，避免激活或 Explorer 重启后出现任务栏按钮。
#[cfg(windows)]
pub fn configure_pin_window(native_handle: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    // SAFETY: 调用方传入主线程上仍存活的 HWND；只修改扩展样式，不转移所有权。
    unsafe {
        let hwnd = native_handle as _;
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (style & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
        );
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
    }
}

#[cfg(all(windows, debug_assertions))]
pub fn pin_window_hides_taskbar(native_handle: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    // SAFETY: 调用方传入当前仍存活的贴图 HWND；仅查询扩展样式。
    let style = unsafe { GetWindowLongPtrW(native_handle as _, GWL_EXSTYLE) };
    style & WS_EX_TOOLWINDOW as isize != 0 && style & WS_EX_APPWINDOW as isize == 0
}

/// 等待桌面合成完成，隐藏后再释放底图或捕获下一次桌面。
pub fn flush_compositor() {
    #[cfg(target_os = "macos")]
    crate::macos::flush_compositor();
    #[cfg(windows)]
    {
        // SAFETY: DwmFlush 无参数、无内存所有权转移。
        let result = unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() };
        if result < 0 {
            log::debug!("等待桌面合成失败：{result:#x}");
        }
    }
}

pub fn cursor_position() -> Option<Pos> {
    #[cfg(target_os = "macos")]
    return crate::macos::cursor_position();
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(windows)]
        {
            let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            // SAFETY: point 是有效且可写的 POINT。
            if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) } != 0
            {
                return Some(Pos::new(point.x as f32, point.y as f32));
            }
        }
        None
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(target_os = "windows")]
mod imp {
    use super::{Pos, Rect};

    use std::ffi::c_void;
    use std::mem::size_of;

    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetWindow, GetWindowRect, GetWindowThreadProcessId,
        IsWindowVisible, WindowFromPoint, GA_ROOT, GW_HWNDNEXT,
    };

    /// 这些窗口类代表"桌面本身"，不该被当成可截图的窗口。
    ///
    /// `Progman` 是桌面，"显示桌面"之后前台会换成 `WorkerW`。
    /// 命中它们时返回 `None`，让上层退回整屏 —— 这比"把整个桌面当成一个
    /// 窗口"更符合预期（用户想截桌面时，整屏正好就是他想要的）。
    const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];

    pub fn window_under_point(point: Pos) -> Option<Rect> {
        // Win32 用的是 i32 整数坐标；子像素位置直接截断即可
        let pt = POINT {
            x: point.x as i32,
            y: point.y as i32,
        };
        unsafe { hit_test_walk(pt) }
    }

    /// 从指针处最上层的窗口开始，沿 Z 序往下找第一个"合格"的窗口。
    ///
    /// **为什么不能直接用 `WindowFromPoint` 的结果**：我们的遮罩窗口是全屏 +
    /// 置顶的，它必然盖在所有东西上面，所以 `WindowFromPoint` 永远返回它自己。
    /// 必须跳过本进程的窗口，一路往下 `GW_HWNDNEXT` 找。
    unsafe fn hit_test_walk(pt: POINT) -> Option<Rect> {
        let own_pid = unsafe { GetCurrentProcessId() };
        let mut hwnd = unsafe { WindowFromPoint(pt) };

        while !hwnd.is_null() {
            // 命中的通常是子窗口（按钮、编辑框……），要提升到顶层窗口
            let root = unsafe {
                let a = GetAncestor(hwnd, GA_ROOT);
                if a.is_null() {
                    hwnd
                } else {
                    a
                }
            };

            if unsafe { is_eligible(root, own_pid) } {
                if let Some(rect) = unsafe { window_bounds(root) } {
                    if !rect.contains(Pos::new(pt.x as f32, pt.y as f32)) {
                        hwnd = unsafe { GetWindow(root, GW_HWNDNEXT) };
                        continue;
                    }
                    log::debug!(
                        "窗口吸附命中 hwnd={root:p} rect=({:.0},{:.0})-({:.0},{:.0})",
                        rect.min.x,
                        rect.min.y,
                        rect.max.x,
                        rect.max.y
                    );
                    return Some(rect);
                }
            }

            // 沿 Z 序往下走一层
            hwnd = unsafe { GetWindow(root, GW_HWNDNEXT) };
        }

        log::debug!("指针下没有可吸附的窗口（可能是桌面或只剩本程序自己）");
        None
    }

    /// 这个窗口够不够格当选区。
    unsafe fn is_eligible(hwnd: HWND, own_pid: u32) -> bool {
        // ① 不是自己的窗口（遮罩、未来的贴图窗口都属于本进程）
        if unsafe { window_pid(hwnd) } == own_pid {
            return false;
        }
        // ② 可见
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return false;
        }
        // ③ 没被 DWM 隐藏
        //
        // 这一条比 `IsWindowVisible` 重要得多：UWP 应用被挂起、以及窗口位于
        // **其他虚拟桌面**上时，`IsWindowVisible` 仍然返回真，只有 DWM 的
        // cloaked 标志能告诉我们"它其实不在屏幕上"。漏掉这条会吸附到
        // 别的虚拟桌面里的窗口，用户完全看不懂发生了什么。
        if unsafe { is_cloaked(hwnd) } {
            return false;
        }
        // ④ 不是桌面本身
        if unsafe { is_desktop(hwnd) } {
            return false;
        }
        true
    }

    unsafe fn window_pid(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        pid
    }

    unsafe fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked: u32 = 0;
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                &mut cloaked as *mut u32 as *mut c_void,
                size_of::<u32>() as u32,
            )
        };
        // HRESULT：>= 0 才算成功。查询失败时保守地认为"没被隐藏"，
        // 否则某些老程序会被整片跳过。
        hr >= 0 && cloaked != 0
    }

    unsafe fn is_desktop(hwnd: HWND) -> bool {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return false;
        }
        let name = String::from_utf16_lossy(&buf[..n as usize]);
        DESKTOP_CLASSES.contains(&name.as_str())
    }

    /// 取窗口的**视觉**边界。
    ///
    /// 用 `DWMWA_EXTENDED_FRAME_BOUNDS` 而不是 `GetWindowRect`：后者包含
    /// 不可见的窗口阴影/拉伸边框，在 Win10+ 上会比肉眼看到的窗口大一圈，
    /// 直接拿来截图会带进一片背景色。
    ///
    /// 查询失败（部分无边框窗口、非 DWM 环境）时退回 `GetWindowRect`。
    unsafe fn window_bounds(hwnd: HWND) -> Option<Rect> {
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                &mut rc as *mut RECT as *mut c_void,
                size_of::<RECT>() as u32,
            )
        };
        if hr < 0 && unsafe { GetWindowRect(hwnd, &mut rc) } == 0 {
            return None;
        }

        // 最小化时 Win32 会把窗口矩形丢到 (-32000, -32000) 这种地方，
        // 这种坐标绝不能拿来做选区。
        if rc.right <= rc.left || rc.bottom <= rc.top {
            return None;
        }

        Some(Rect::from_points(
            Pos::new(rc.left as f32, rc.top as f32),
            Pos::new(rc.right as f32, rc.bottom as f32),
        ))
    }
}

// ---------------------------------------------------------------- 其他平台

#[cfg(target_os = "macos")]
mod imp {
    pub use crate::macos::window_under_point;
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::{Pos, Rect};

    /// Linux 暂未实现；上层回退到屏幕预选。
    pub fn window_under_point(_point: Pos) -> Option<Rect> {
        None
    }
}
