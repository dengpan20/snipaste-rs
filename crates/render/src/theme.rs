//! 应用的视觉主题。
//!
//! 只有一处定义颜色，方便后面做浅色 / 深色切换与高对比度适配。

use egui::{Color32, Context, Visuals};

/// 主强调色（选中工具、选区边框）。
pub const ACCENT: Color32 = Color32::from_rgb(56, 152, 236);

/// 工具条底色（半透明深灰，压在截图之上不会太抢眼）。
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(250, 250, 250);

/// 工具条前景色。
pub const TOOLBAR_FG: Color32 = Color32::from_rgb(48, 51, 56);

/// 次要文字 / 未选中图标。
pub const TOOLBAR_DIM: Color32 = Color32::from_rgb(108, 113, 120);

/// 悬浮态背景。
pub const HOVER_BG: Color32 = Color32::from_rgb(225, 238, 250);

/// 选区外部的压暗遮罩。
///
/// Snipaste 的标志性观感就来自这一层：非选区部分被压暗，视线自然聚焦到选区。
pub const DIM_OVERLAY: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 120);

/// 未能实现的工具在按钮上打的提示色。
pub const WIP: Color32 = Color32::from_rgb(214, 158, 66);

/// 应用主题到 egui 上下文。
///
/// egui 0.36 起 `Context::style()` 被移除，改为按主题（明/暗）分别存样式，
/// 因此这里用 `all_styles_mut` 一次性改掉两个主题的间距 —— 以后加浅色模式时
/// 不需要再补一遍。
pub fn apply(ctx: &Context) {
    let mut visuals = Visuals::light();
    visuals.panel_fill = TOOLBAR_BG;
    visuals.window_fill = TOOLBAR_BG;
    visuals.window_stroke = egui::Stroke::new(1.0, Color32::from_rgb(70, 74, 82));
    visuals.override_text_color = Some(TOOLBAR_FG);
    visuals.selection.bg_fill = ACCENT;
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        // 截图工具条应在选区确定后立即完整显示。
        style.animation_time = 0.0;
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(10);
    });
}
