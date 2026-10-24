//! Shared screenshot/pin saving with a persistent count for each local date.
use crate::settings::{ImageFormat, Preferences, SettingsStore};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
struct SaveCounts {
    days: BTreeMap<String, u64>,
}

struct SaveSequence {
    path: PathBuf,
    counts: SaveCounts,
    date: String,
    next: u64,
    // Released on cancellation, failure, or success. Multiple app instances
    // cannot offer the same sequence while another save dialog is open.
    _lock: File,
}

impl SaveSequence {
    fn open(path: &Path, date: &str) -> Result<Self> {
        let parent = path.parent().context("保存计数目录无效")?;
        std::fs::create_dir_all(parent).context("无法创建保存计数目录")?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))
            .context("无法打开保存计数锁")?;
        lock.try_lock()
            .context("另一个保存窗口正在使用截图序号，请完成后重试")?;
        let counts: SaveCounts = match std::fs::read(path) {
            Ok(contents) => serde_json::from_slice(&contents).context("保存计数文件格式无效")?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => SaveCounts::default(),
            Err(err) => return Err(err).context("无法读取保存计数"),
        };
        let next = counts
            .days
            .get(date)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .context("当天保存序号已达上限")?;
        Ok(Self {
            path: path.to_owned(),
            counts,
            date: date.to_owned(),
            next,
            _lock: lock,
        })
    }

    fn filename(&self, extension: &str) -> String {
        format!("截图-{}-{}.{}", self.date, self.next, extension)
    }

    fn record_saved(&mut self) -> Result<()> {
        self.counts.days.insert(self.date.clone(), self.next);
        let contents = serde_json::to_vec(&self.counts)?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let mut file = File::create(&temporary)?;
            file.write_all(&contents)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.context("图片已保存，但无法更新当天保存计数")
    }
}

pub fn save_image(image: &image::RgbaImage, preferences: &Preferences) -> Result<bool> {
    let date = snipaste_platform::dialog::local_date().map_err(anyhow::Error::msg)?;
    let counts_path = SettingsStore::for_user()?.save_counts_path();
    let directory = (!preferences.save_directory.trim().is_empty())
        .then(|| Path::new(preferences.save_directory.trim()));
    save_with_dialog(image, preferences, &counts_path, &date, |filename| {
        snipaste_platform::dialog::save_image_path_with_name(
            directory,
            preferences.image_format == ImageFormat::Jpeg,
            filename,
        )
        .map_err(anyhow::Error::msg)
    })
}

fn save_with_dialog(
    image: &image::RgbaImage,
    preferences: &Preferences,
    counts_path: &Path,
    date: &str,
    choose_path: impl FnOnce(&str) -> Result<Option<PathBuf>>,
) -> Result<bool> {
    let mut sequence = SaveSequence::open(counts_path, date)?;
    let filename = sequence.filename(preferences.image_format.extension());
    let Some(mut path) = choose_path(&filename)? else {
        return Ok(false);
    };
    if path.extension().is_none() {
        path.set_extension(preferences.image_format.extension());
    }
    let format = image::ImageFormat::from_path(&path)?;
    if format == image::ImageFormat::Jpeg {
        use image::buffer::ConvertBuffer;
        let rgb: image::RgbImage = image.convert();
        rgb.save_with_format(&path, format)?;
    } else {
        image.save_with_format(&path, format)?;
    }
    sequence.record_saved()?;
    log::info!("截图已保存：{}", path.display());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("yingtie-save-{}-{nonce}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn counts(&self) -> PathBuf {
            self.0.join("save-counts.json")
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn image() -> image::RgbaImage {
        image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]))
    }

    #[test]
    fn successful_saves_share_daily_count_across_formats_directories_and_restarts() {
        let directory = TestDirectory::new();
        for (count, format, extension) in
            [(1, ImageFormat::Png, "png"), (2, ImageFormat::Jpeg, "jpg")]
        {
            let destination = directory.0.join(count.to_string());
            std::fs::create_dir(&destination).unwrap();
            let prefs = Preferences {
                image_format: format,
                ..Default::default()
            };
            assert!(
                save_with_dialog(&image(), &prefs, &directory.counts(), "20261024", |name| {
                    assert_eq!(name, format!("截图-20261024-{count}.{extension}"));
                    Ok(Some(destination.join(name)))
                })
                .unwrap()
            );
            let saved = image::open(destination.join(format!("截图-20261024-{count}.{extension}")))
                .unwrap();
            assert_eq!((saved.width(), saved.height()), (8, 4));
        }
        // Re-open the disk state as a newly started process would.
        let sequence = SaveSequence::open(&directory.counts(), "20261024").unwrap();
        assert_eq!(sequence.filename("png"), "截图-20261024-3.png");
    }

    #[test]
    fn cancellation_and_failed_writes_do_not_consume_a_number() {
        let directory = TestDirectory::new();
        let prefs = Preferences::default();
        assert!(
            !save_with_dialog(&image(), &prefs, &directory.counts(), "20261024", |_| Ok(
                None
            ))
            .unwrap()
        );
        assert!(
            save_with_dialog(&image(), &prefs, &directory.counts(), "20261024", |name| {
                Ok(Some(directory.0.join("missing-directory").join(name)))
            })
            .is_err()
        );
        assert_eq!(
            SaveSequence::open(&directory.counts(), "20261024")
                .unwrap()
                .filename("png"),
            "截图-20261024-1.png"
        );
        // A custom name still counts as a successful save; no extension uses the preference.
        assert!(
            save_with_dialog(&image(), &prefs, &directory.counts(), "20261024", |_| {
                Ok(Some(directory.0.join("自定义文件名")))
            })
            .unwrap()
        );
        assert!(directory.0.join("自定义文件名.png").is_file());
        assert_eq!(
            SaveSequence::open(&directory.counts(), "20261024")
                .unwrap()
                .filename("png"),
            "截图-20261024-2.png"
        );
    }

    #[test]
    fn each_day_starts_at_one_and_keeps_its_own_saved_count() {
        let directory = TestDirectory::new();
        for date in ["20261024", "20261025"] {
            let mut sequence = SaveSequence::open(&directory.counts(), date).unwrap();
            assert_eq!(sequence.filename("png"), format!("截图-{date}-1.png"));
            sequence.record_saved().unwrap();
        }
        assert_eq!(
            SaveSequence::open(&directory.counts(), "20261024")
                .unwrap()
                .filename("png"),
            "截图-20261024-2.png"
        );
    }

    #[test]
    fn concurrent_save_dialogs_cannot_offer_the_same_number() {
        let directory = TestDirectory::new();
        let sequence = SaveSequence::open(&directory.counts(), "20261024").unwrap();
        assert!(SaveSequence::open(&directory.counts(), "20261024").is_err());
        drop(sequence);
        assert!(SaveSequence::open(&directory.counts(), "20261024").is_ok());
    }
}
