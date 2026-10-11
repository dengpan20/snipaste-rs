//! # snipaste-render
//!
//! 渲染层：一切"把领域模型变成像素"的知识都收在这里。
//!
//! 本 crate **不创建窗口、不碰 wgpu device、不做事件循环** ——
//! 它只依赖 `egui` 的 `Context` / `Painter`，因此可以脱离窗口单独测试
//! （用 `egui::Context::run` 跑一帧即可断言生成的图元数量）。
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`fonts`] | 中文字体探测与注册（不把字体打进二进制） |
//! | [`theme`] | 视觉主题：颜色、间距、遮罩透明度 |
//! | [`view`]  | VSS 物理像素 ↔ egui 逻辑点的换算 |
//! | [`canvas`]| `Document` → egui `Shape` 的绘制桥 |
//!
//! 实时马赛克读取冻结底图；文字输入在 app 层由 egui 承载，导出使用同一字体。

pub mod canvas;
pub mod fonts;
pub mod theme;
pub mod view;

pub use canvas::{
    paint_annotation, paint_dim_outside, paint_document, paint_selection_frame, to_color32,
};
pub use fonts::Candidate as FontCandidate;
pub use theme as colors;
pub use view::Viewport;
