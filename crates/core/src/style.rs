//! 工具、颜色与样式定义。

use crate::geometry::Pos;

/// RGBA 颜色。全部按**非预乘**存储，与 egui 的 `Color32` 语义一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// 转为 egui 需要的 `[f32; 4]`（sRGB 线性空间，0.0 ~ 1.0）。
    pub fn to_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    /// 调整透明度（0.0 ~ 1.0）。记号笔用它把实色变成半透明高亮色。
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub fn with_alpha_f(self, a: f32) -> Self {
        let clamped = a.clamp(0.0, 1.0);
        Self {
            a: (clamped * 255.0).round() as u8,
            ..self
        }
    }

    /// 是否完全透明 —— 完全透明的填充层不应该参与渲染。
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }
}

/// 马赛克的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MosaicMode {
    /// 像素化：块内取平均后平铺，最常用的马赛克。
    Pixelate,
    /// 模糊：盒式/高斯模糊，观感更柔和。
    Blur,
}

/// 当前激活的标注工具。
///
/// 顺序即工具栏顺序，也即 `Tab` 键的循环顺序 —— 见 [`Tool::ALL`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// 指针 / 移动选区与已有图元，不产生新标注。
    Select,
    Rect,
    Ellipse,
    Arrow,
    Pen,
    Marker,
    Mosaic,
    Text,
    Eraser,
}

impl Tool {
    /// 工具栏显示顺序，同时也是 `Tab` 的循环顺序。
    ///
    /// 刻意把高频工具（矩形 / 箭头）排在前面：截图标注场景下这两个占了绝大多数使用量。
    pub const ALL: [Tool; 9] = [
        Tool::Select,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Pen,
        Tool::Marker,
        Tool::Mosaic,
        Tool::Text,
        Tool::Eraser,
    ];

    /// 工具是否通过"拖拽出一个矩形"来产生图元。
    ///
    /// 决定鼠标按下时是走 `draft`（拖拽出图元）还是走别的事件分支
    /// （`Text` 走文本框；马赛克/橡皮擦使用矩形或笔刷草稿）。
    pub fn is_drag_shape(self) -> bool {
        matches!(
            self,
            Tool::Rect
                | Tool::Ellipse
                | Tool::Arrow
                | Tool::Pen
                | Tool::Marker
                | Tool::Mosaic
                | Tool::Eraser
        )
    }

    /// 工具是否按采样点累积成路径（决定是否需要记录轨迹点）。
    pub fn is_freehand(self) -> bool {
        matches!(self, Tool::Pen | Tool::Marker)
    }

    /// 中文显示名。
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "指针",
            Tool::Rect => "矩形",
            Tool::Ellipse => "椭圆",
            Tool::Arrow => "箭头",
            Tool::Pen => "画笔",
            Tool::Marker => "记号笔",
            Tool::Mosaic => "马赛克",
            Tool::Text => "文字",
            Tool::Eraser => "橡皮擦",
        }
    }

    /// 默认快捷键（单字符，显示在工具按钮角标上）。
    pub fn shortcut(self) -> Option<char> {
        match self {
            Tool::Select => Some('V'),
            Tool::Rect => Some('R'),
            Tool::Ellipse => Some('O'),
            Tool::Arrow => Some('A'),
            Tool::Pen => Some('P'),
            Tool::Marker => Some('H'),
            Tool::Mosaic => Some('M'),
            Tool::Text => Some('T'),
            Tool::Eraser => Some('E'),
        }
    }

    /// `Tab` 切换：循环到下一个工具。
    pub fn next(self) -> Tool {
        let idx = Tool::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Tool::ALL[(idx + 1) % Tool::ALL.len()]
    }
}

/// 标注样式。所有尺寸单位都是 **VSS 物理像素**。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub stroke: Color,
    /// 描边宽度（物理像素）。Retina 上同样的视觉粗细需要 2 倍的物理像素，
    /// 因此这里存物理像素、由渲染层按显示器的 scale_factor 换算。
    pub stroke_width: f32,
    /// 填充色；`None` 表示空心。
    pub fill: Option<Color>,
    /// 文字字号（物理像素）。
    pub font_size: f32,
    /// 马赛克块大小（物理像素），有效范围见 [`Style::MOSAIC_BLOCK_RANGE`]。
    pub mosaic_block: u32,
    pub mosaic_mode: MosaicMode,
}

impl Style {
    /// 马赛克块大小的可调范围。下限保证隐私遮盖有效，上限保证不至于糊成一片。
    pub const MOSAIC_BLOCK_RANGE: (u32, u32) = (4, 40);

    /// 笔宽可调范围（物理像素）。
    pub const STROKE_WIDTH_RANGE: (f32, f32) = (1.0, 32.0);

    /// 调节笔宽。`delta > 0` 变粗。按比例增长而不是线性增长，
    /// 这样从 1px 到 2px 与从 16px 到 32px 的手感一致。
    pub fn nudge_stroke_width(&mut self, delta: f32) {
        let factor = if delta > 0.0 { 1.2 } else { 1.0 / 1.2 };
        let (min, max) = Self::STROKE_WIDTH_RANGE;
        self.stroke_width = (self.stroke_width * factor).clamp(min, max);
    }

    /// 调节马赛克块大小。
    pub fn nudge_mosaic_block(&mut self, delta: i32) {
        let (min, max) = Self::MOSAIC_BLOCK_RANGE;
        let next = self.mosaic_block as i32 + delta * 2;
        self.mosaic_block = next.clamp(min as i32, max as i32) as u32;
    }

    /// 为记号笔生成样式：笔尖加粗、颜色半透明。
    ///
    /// 这里把记号笔的样式差异集中在一个函数里，避免散落在各处的魔法数字。
    pub fn for_marker(stroke: Color, stroke_width: f32) -> Self {
        Self {
            stroke: stroke.with_alpha_f(0.4),
            // 记号笔笔尖默认为画笔的 3 倍
            stroke_width: (stroke_width * 3.0)
                .clamp(Self::STROKE_WIDTH_RANGE.0, Self::STROKE_WIDTH_RANGE.1),
            fill: None,
            ..Self::default()
        }
    }

    /// 沿路径 / 矩形的中心线命中测试所需的最小容差。
    ///
    /// 细线很难点中，因此给一个下限（6 物理像素），保证 1px 的线也能被舒服地擦掉。
    pub fn hit_tolerance(&self) -> f32 {
        (self.stroke_width * 0.5).max(6.0)
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: Color::rgb(232, 66, 66), // 默认红色：截图上最醒目
            stroke_width: 3.0,
            fill: None,
            font_size: 18.0,
            mosaic_block: 12,
            mosaic_mode: MosaicMode::Pixelate,
        }
    }
}

/// 常用色板（工具栏快速选色）。
pub const PALETTE: [Color; 8] = [
    Color::rgb(232, 66, 66),   // 红
    Color::rgb(240, 160, 32),  // 橙
    Color::rgb(246, 214, 62),  // 黄
    Color::rgb(70, 190, 110),  // 绿
    Color::rgb(56, 152, 236),  // 蓝
    Color::rgb(150, 92, 220),  // 紫
    Color::rgb(255, 255, 255), // 白（深色背景上很常用）
    Color::rgb(0, 0, 0),       // 黑
];

/// 箭头头部几何参数。
///
/// 抽成函数是为了保证"预览"与"导出"两条路径用到完全相同的数字 ——
/// 任何一处分叉都会导致所见非所得。
pub struct ArrowHead {
    /// 两个后掠角顶点的位置。
    pub left: Pos,
    pub right: Pos,
    pub tip: Pos,
}

impl ArrowHead {
    /// 依据箭杆方向生成箭头头部。
    ///
    /// - `stroke_width` 用于计算头部展开尺寸，保证任何笔宽下比例都协调
    /// - 半角固定 150°，即头部后掠 30°
    pub fn build(from: Pos, to: Pos, stroke_width: f32) -> Option<Self> {
        let dir = to - from;
        let len = dir.length();
        if len < f32::EPSILON {
            return None;
        }
        let (ux, uy) = (dir.x / len, dir.y / len);
        // 头部长度随笔宽线性增长，但不短于 8px，避免细箭头看起来没有头
        let head_len = (stroke_width * 4.0).max(8.0);
        // 半展开宽度 ≈ head_len * tan(30°)
        let half_w = head_len * 0.5773;

        let base = Pos::new(to.x - ux * head_len, to.y - uy * head_len);
        Some(Self {
            tip: to,
            left: Pos::new(base.x - uy * half_w, base.y + ux * half_w),
            right: Pos::new(base.x + uy * half_w, base.y - ux * half_w),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_cycle_wraps_around() {
        assert_eq!(Tool::Select.next(), Tool::Rect);
        assert_eq!(Tool::Eraser.next(), Tool::Select);
        // 循环 ALL 长度次应回到原点
        let mut t = Tool::Rect;
        for _ in 0..Tool::ALL.len() {
            t = t.next();
        }
        assert_eq!(t, Tool::Rect);
    }

    #[test]
    fn tool_classification() {
        assert!(Tool::Rect.is_drag_shape());
        assert!(Tool::Mosaic.is_drag_shape());
        // 文字、橡皮擦不通过"拖出矩形"产生图元；二维码是应用命令。
        assert!(!Tool::Text.is_drag_shape());
        assert!(Tool::Eraser.is_drag_shape());
        // 指针也不产生图元
        assert!(!Tool::Select.is_drag_shape());
    }

    #[test]
    fn freehand_only_pen_and_marker() {
        assert!(Tool::Pen.is_freehand());
        assert!(Tool::Marker.is_freehand());
        assert!(!Tool::Rect.is_freehand());
    }

    #[test]
    fn marker_style_is_translucent_and_thick() {
        let base = Style::default();
        let m = Style::for_marker(base.stroke, base.stroke_width);
        assert!(m.stroke.a < 255, "记号笔必须半透明");
        assert!(m.stroke_width > base.stroke_width, "记号笔必须更粗");
    }

    #[test]
    fn stroke_width_nudge_is_multiplicative_and_clamped() {
        let mut s = Style {
            stroke_width: 10.0,
            ..Default::default()
        };
        s.nudge_stroke_width(1.0);
        assert!((s.stroke_width - 12.0).abs() < 1e-3);
        s.nudge_stroke_width(-1.0);
        assert!((s.stroke_width - 10.0).abs() < 1e-3);

        // 上界钳制
        for _ in 0..100 {
            s.nudge_stroke_width(1.0);
        }
        assert_eq!(s.stroke_width, Style::STROKE_WIDTH_RANGE.1);

        // 下界钳制
        for _ in 0..200 {
            s.nudge_stroke_width(-1.0);
        }
        assert_eq!(s.stroke_width, Style::STROKE_WIDTH_RANGE.0);
    }

    #[test]
    fn mosaic_block_clamped() {
        let mut s = Style::default();
        for _ in 0..100 {
            s.nudge_mosaic_block(1);
        }
        assert_eq!(s.mosaic_block, Style::MOSAIC_BLOCK_RANGE.1);
        for _ in 0..100 {
            s.nudge_mosaic_block(-1);
        }
        assert_eq!(s.mosaic_block, Style::MOSAIC_BLOCK_RANGE.0);
    }

    #[test]
    fn hit_tolerance_has_floor() {
        let mut s = Style {
            stroke_width: 1.0,
            ..Default::default()
        };
        // 1px 的线也要有 6px 的可点击容差
        assert_eq!(s.hit_tolerance(), 6.0);
        s.stroke_width = 20.0;
        assert_eq!(s.hit_tolerance(), 10.0);
    }

    #[test]
    fn arrow_head_geometry() {
        let head = ArrowHead::build(Pos::new(0.0, 0.0), Pos::new(100.0, 0.0), 3.0).unwrap();
        assert_eq!(head.tip, Pos::new(100.0, 0.0));
        // 头部沿 -x 方向后掠
        assert!((head.left.x - head.right.x).abs() < 1e-4);
        assert!(head.left.y > 0.0 && head.right.y < 0.0);

        // 零长度箭头不产生头部
        assert!(ArrowHead::build(Pos::ZERO, Pos::ZERO, 3.0).is_none());
    }

    #[test]
    fn arrow_head_scales_with_stroke_width() {
        let thin = ArrowHead::build(Pos::ZERO, Pos::new(100.0, 0.0), 2.0).unwrap();
        let thick = ArrowHead::build(Pos::ZERO, Pos::new(100.0, 0.0), 10.0).unwrap();
        let thin_w = (thin.left.y - thin.right.y).abs();
        let thick_w = (thick.left.y - thick.right.y).abs();
        assert!(thick_w > thin_w, "笔宽越大箭头头应越大");
    }
}
