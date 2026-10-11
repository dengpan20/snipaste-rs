//! # snipaste-core
//!
//! 领域模型层。**本 crate 不允许出现任何平台相关代码**：
//!
//! - 不出现 `#[cfg(target_os = ...)]`
//! - 不出现文件系统、网络、窗口、GPU 调用
//! - 只依赖 `std` 的纯计算部分（f32 / Vec / String）
//!
//! 这样做的收益：全部业务规则都能用单元测试覆盖，不依赖截图、不依赖显示器，
//! CI 上跑得飞快；平台差异被迫收敛到 `snipaste-platform` 与 `snipaste-capture`。
//!
//! ## 坐标系约定（最重要的一条）
//!
//! 本 crate 中所有坐标都是 **VSS（Virtual Screen Space）物理像素**：
//!
//! - 单位：**物理像素**（不是逻辑点 / DIP）
//! - 原点：**虚拟桌面左上角**（所有显示器并集的外接矩形左上角）
//! - 轴向：x 向右、y 向下，与屏幕一致
//!
//! 只有在把内容画到某一块具体显示器上时，才通过 [`VssToScreen`] 转换成该屏的
//! 逻辑坐标。混用逻辑坐标与物理坐标是多屏 DPI 问题的唯一根源，因此在类型上
//! 就把它隔离掉。

pub mod annotation;
pub mod document;
pub mod geometry;
pub mod history;
pub mod simplify;
pub mod style;
pub mod vss;

pub use annotation::{Annotation, ArrowKind, MosaicRegion, PathKind};
pub use document::Document;
pub use geometry::{Handle, Pos, Rect, Transform, HANDLE_HIT_RADIUS, HANDLE_HIT_SLOP, HANDLE_SIZE};
pub use history::{Edit, History};
pub use simplify::{catmull_rom_to_bezier, douglas_peucker};
pub use style::{Color, MosaicMode, Style, Tool};
pub use vss::{MonitorInfo, VssLayout, VssToScreen};
