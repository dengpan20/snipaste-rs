//! 只读的进程内存测量；不裁剪工作集，避免以换页伪装资源优化。
#[derive(Debug)]
pub struct Usage {
    pub resident: usize,
    pub private: usize,
    pub peak_resident: usize,
    pub peak_private: usize,
}

#[cfg(windows)]
pub fn usage() -> Option<Usage> {
    use windows_sys::Win32::System::{
        ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::GetCurrentProcess,
    };
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        counters.cb = size;
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        ) == 0
        {
            return None;
        }
        Some(Usage {
            resident: counters.WorkingSetSize,
            private: counters.PrivateUsage,
            peak_resident: counters.PeakWorkingSetSize,
            peak_private: counters.PeakPagefileUsage,
        })
    }
}

#[cfg(target_os = "macos")]
pub fn usage() -> Option<Usage> {
    crate::macos::memory_usage()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn usage() -> Option<Usage> {
    None
}
