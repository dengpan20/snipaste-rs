//! 中文字体发现与注册。
//!
//! ## 为什么不把字体打进二进制
//!
//! 中文字体文件普遍 10 MB 以上（本机实测：等线 16.3 MB、微软雅黑 19.7 MB、
//! 思源黑体 17.8 MB）。静态链接进 exe 会让安装包与常驻内存同时变差，
//! 与"占用内存小"的硬性要求正中冲突。
//!
//! Windows 与 macOS 都自带质量相当不错的中文字体，因此运行时探测即可。
//! 若发行版需要"跨平台完全一致的观感"，把字体丢进 `assets/fonts/` 就行 ——
//! 本模块会**优先使用内置字体**。
//!
//! ## 关于 TTC
//!
//! `msyh.ttc`（微软雅黑）、`PingFang.ttc`（苹方）这类是字体集合，一个文件里
//! 有多个 face，必须靠 [`egui::FontData::index`] 选中其中一个，否则会加载到
//! 字重不对的那个。epaint 0.36 的字体后端已换成 `skrifa`（Fontations），
//! 对集合与可变字体都支持良好。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use egui::{FontData, FontDefinitions, FontFamily};

/// 编译期记录的开发期资源目录（仅用于 `cargo run`，发行时靠 exe 同级的 `assets/`）。
const DEV_ASSETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts");

/// 一个候选字体文件。
#[derive(Debug, Clone)]
pub struct Candidate {
    /// 给人看的名字（日志 / 关于界面用）。
    pub label: String,
    pub path: PathBuf,
    /// 字体集合中的 face 下标；普通 TTF / OTF 恒为 0。
    pub index: u32,
}

impl Candidate {
    fn new(label: impl Into<String>, path: impl Into<PathBuf>, index: u32) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            index,
        }
    }
}

/// 可用的字体扩展名。
fn is_font_file(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ttf" | "otf" | "ttc" | "otc")
    )
}

/// 收集内置字体目录里的字体。
///
/// 发行包里放 `assets/fonts/`，开发时放项目根的 `assets/fonts/`，两处都会看。
fn bundled_candidates() -> Vec<Candidate> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("assets").join("fonts"));
        }
    }
    dirs.push(PathBuf::from(DEV_ASSETS_DIR));

    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && is_font_file(p))
            .collect();
        // 排序保证同一台机器上每次启动选中的字体一致
        files.sort();
        for path in files {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("font")
                .to_owned();
            out.push(Candidate::new(format!("内置字体 {name}"), path, 0));
        }
    }
    out
}

/// 系统字体候选（Windows）。
///
/// 顺序即优先级。等线（Deng）排第一的理由：它是 Windows 10 起的现代中文
/// 无衬线字体，观感接近苹方，且是纯 TTF（无集合下标歧义）。
#[cfg(target_os = "windows")]
fn system_candidates() -> Vec<Candidate> {
    const LIST: &[(&str, &str, u32)] = &[
        ("等线 Deng", "Deng.ttf", 0),
        ("思源黑体 Noto Sans SC", "NotoSansSC-VF.ttf", 0),
        ("微软雅黑 Microsoft YaHei", "msyh.ttc", 0),
        ("黑体 SimHei", "simhei.ttf", 0),
        ("微软正黑体 Microsoft JhengHei", "msjh.ttc", 0),
        ("宋体 SimSun", "simsun.ttc", 0),
    ];
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
    let fonts = Path::new(&root).join("Fonts");
    LIST.iter()
        .map(|(label, file, index)| Candidate::new(*label, fonts.join(file), *index))
        .collect()
}

/// 系统字体候选（macOS）。
#[cfg(target_os = "macos")]
fn system_candidates() -> Vec<Candidate> {
    const LIST: &[(&str, &str, u32)] = &[
        ("苹方 PingFang SC", "/System/Library/Fonts/PingFang.ttc", 0),
        (
            "华文黑体 STHeiti Medium",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            0,
        ),
        (
            "冬青黑体 Hiragino Sans GB",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            0,
        ),
        ("Arial Unicode MS", "/Library/Fonts/Arial Unicode.ttf", 0),
    ];
    let mut candidates = Vec::new();
    // Modern macOS may store PingFang in AssetsV2 rather than Fonts. CoreText
    // resolves the installed face without scanning a large system directory.
    if let Some(path) = snipaste_platform::macos::chinese_font_path() {
        candidates.push(Candidate::new("苹方 PingFang SC", path, 0));
    }
    candidates.extend(
        LIST.iter()
            .map(|(label, file, index)| Candidate::new(*label, PathBuf::from(file), *index)),
    );
    candidates
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_candidates() -> Vec<Candidate> {
    Vec::new()
}

/// 全部候选，按优先级从高到低。
pub fn candidates() -> Vec<Candidate> {
    let mut all = bundled_candidates();
    all.extend(system_candidates());
    all
}

/// 第一个真实存在的候选字体。
pub fn find() -> Option<Candidate> {
    candidates().into_iter().find(|c| c.path.is_file())
}

/// 把中文字体注册进 egui，返回实际用上的那个。
///
/// 中文字体会被插到 `Proportional` 与 `Monospace` 两个族的**最前面**，
/// 让中英文都用同一个字体渲染，观感更统一；egui 自带的拉丁字体与 emoji
/// 字体保留在后面作为兜底（emoji、罕见符号仍能正常显示）。
///
/// 必须在第一帧之前调用。找不到任何中文字体时返回 `None` 并打警告 ——
/// 此时界面里的中文会显示成豆腐块，但程序不会崩。
pub fn install(ctx: &egui::Context) -> Option<Candidate> {
    let Some(cand) = find() else {
        log::warn!(
            "未找到任何中文字体，界面中的中文将显示为方块。\
             可在 assets/fonts/ 放置一个 .ttf/.otf 解决。"
        );
        return None;
    };

    // 上下文之间共享字体文件；弱引用不阻止最后一个界面关闭时释放文件内存。
    static FONT: Mutex<Option<(PathBuf, u32, Weak<FontData>)>> = Mutex::new(None);
    let data = {
        let mut cache = FONT.lock().unwrap_or_else(|err| err.into_inner());
        let shared = cache.as_ref().and_then(|(path, index, data)| {
            (path == &cand.path && *index == cand.index)
                .then(|| data.upgrade())
                .flatten()
        });
        if let Some(data) = shared {
            data
        } else {
            let loaded = {
                #[cfg(target_os = "macos")]
                {
                    let compact = (|| {
                        let mut file = std::fs::File::open(&cand.path)?;
                        crate::font_file::load(&mut file, cand.path.metadata()?.len(), cand.index)
                    })();
                    compact.or_else(|err| {
                        log::warn!("读取单个字体失败，使用原始字体集合：{err}");
                        std::fs::read(&cand.path).map(|bytes| (bytes, cand.index))
                    })
                }
                #[cfg(not(target_os = "macos"))]
                std::fs::read(&cand.path).map(|bytes| (bytes, cand.index))
            };
            let (bytes, index) = match loaded {
                Ok(font) => font,
                Err(err) => {
                    log::warn!("读取字体 {} 失败：{err}", cand.path.display());
                    return None;
                }
            };
            log::info!("中文字体数据：{} bytes", bytes.len());
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            let data = Arc::new(data);
            *cache = Some((cand.path.clone(), cand.index, Arc::downgrade(&data)));
            data
        }
    };

    const KEY: &str = "cjk";
    let mut defs = FontDefinitions::default();
    defs.font_data.insert(KEY.to_owned(), data);
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(family).or_default();
        if !list.iter().any(|n| n == KEY) {
            list.insert(0, KEY.to_owned());
        }
    }
    ctx.set_fonts(defs);

    log::info!(
        "已加载中文字体：{}（{}，face {}）",
        cand.label,
        cand.path.display(),
        cand.index
    );
    Some(cand)
}
