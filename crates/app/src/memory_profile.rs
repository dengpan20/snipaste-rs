//! 显式启用的内存验收。走正常截图路径，记录真实工作集和私有提交量。
use anyhow::Result;
use std::{path::PathBuf, time::Instant};

pub struct Profile {
    pub step: usize,
    pub deadline: Instant,
    pub pins: bool,
    path: PathBuf,
    rows: String,
}

impl Profile {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("SNIPASTE_MEMORY_PROFILE_DIR")?);
        Some(Self {
            step: 0,
            deadline: Instant::now() + std::time::Duration::from_secs(1),
            pins: std::env::var("SNIPASTE_MEMORY_PROFILE_PINS").as_deref() == Ok("1"),
            path: dir.join("memory.csv"),
            rows: "stage,resident_bytes,private_bytes,peak_resident_bytes,peak_private_bytes,capture_ms,gpu,overlay,width,height,pins\n".into(),
        })
    }

    pub fn record(
        &mut self,
        stage: &str,
        capture_ms: u128,
        gpu: bool,
        overlay: bool,
        dimensions: [u32; 2],
        pins: usize,
    ) -> Result<()> {
        let usage = snipaste_platform::memory::usage()
            .ok_or_else(|| anyhow::anyhow!("无法读取进程内存"))?;
        self.rows.push_str(&format!(
            "{stage},{},{},{},{},{capture_ms},{gpu},{overlay},{},{},{pins}\n",
            usage.resident,
            usage.private,
            usage.peak_resident,
            usage.peak_private,
            dimensions[0],
            dimensions[1],
        ));
        std::fs::create_dir_all(self.path.parent().unwrap())?;
        std::fs::write(&self.path, &self.rows)?;
        log::info!("内存验收 {stage}: {usage:?}");
        Ok(())
    }
}
