//! 几何基础类型。**所有坐标均为 VSS 物理像素**，见 crate 根文档。

use std::ops::{Add, Sub};

/// 二维点 / 向量（VSS 物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

impl Pos {
    pub const ZERO: Pos = Pos { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// 欧氏距离。
    pub fn distance(self, other: Pos) -> f32 {
        (self - other).length()
    }

    /// 向量长度。
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// 线性插值。
    pub fn lerp(self, other: Pos, t: f32) -> Pos {
        Pos::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    /// 围绕 `pivot` 旋转（弧度，屏幕坐标系下正值表示顺时针）。
    pub fn rotate_around(self, pivot: Pos, radians: f32) -> Pos {
        let (s, c) = radians.sin_cos();
        let d = self - pivot;
        Pos::new(pivot.x + d.x * c - d.y * s, pivot.y + d.x * s + d.y * c)
    }

    /// 取整到物理像素（导出时对齐用，避免半像素导致的模糊）。
    pub fn round(self) -> Pos {
        Pos::new(self.x.round(), self.y.round())
    }
}

impl Add for Pos {
    type Output = Pos;
    fn add(self, rhs: Pos) -> Pos {
        Pos::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Pos {
    type Output = Pos;
    fn sub(self, rhs: Pos) -> Pos {
        Pos::new(self.x - rhs.x, self.y - rhs.y)
    }
}

/// 轴对齐矩形（VSS 物理像素）。构造后保证 `min <= max`。
///
/// `Default` 得到的是零尺寸矩形（即"空选区"），而不是一块有效区域。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub min: Pos,
    pub max: Pos,
}

impl Rect {
    /// 由任意两个对角点构造，自动归一化（因此支持"从右下往左上拖"）。
    pub fn from_points(a: Pos, b: Pos) -> Self {
        Self {
            min: Pos::new(a.x.min(b.x), a.y.min(b.y)),
            max: Pos::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    /// 由左上角与宽高构造。
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::from_points(Pos::new(x, y), Pos::new(x + w, y + h))
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn size(&self) -> (f32, f32) {
        (self.width(), self.height())
    }

    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub fn center(&self) -> Pos {
        Pos::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
        )
    }

    /// 宽或高为 0 —— 通常意味着"用户只是点了一下没拖动"。
    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    pub fn contains(&self, p: Pos) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    /// 向外扩张（负值即收缩）。
    pub fn expand(&self, d: f32) -> Rect {
        Rect {
            min: Pos::new(self.min.x - d, self.min.y - d),
            max: Pos::new(self.max.x + d, self.max.y + d),
        }
    }

    /// 求交；不相交返回 `None`。
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let r = Rect {
            min: Pos::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Pos::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        };
        if r.is_empty() {
            None
        } else {
            Some(r)
        }
    }

    /// 求并（外接矩形）。
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min: Pos::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Pos::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    /// 平移。
    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect {
            min: Pos::new(self.min.x + dx, self.min.y + dy),
            max: Pos::new(self.max.x + dx, self.max.y + dy),
        }
    }

    /// 把 `self` 平移使其完全落在 `bounds` 内（用于工具栏自动翻转 / 钳制）。
    pub fn clamp_within(&self, bounds: &Rect) -> Rect {
        let mut dx = 0.0;
        let mut dy = 0.0;
        if self.min.x < bounds.min.x {
            dx = bounds.min.x - self.min.x;
        } else if self.max.x > bounds.max.x {
            dx = bounds.max.x - self.max.x;
        }
        if self.min.y < bounds.min.y {
            dy = bounds.min.y - self.min.y;
        } else if self.max.y > bounds.max.y {
            dy = bounds.max.y - self.max.y;
        }
        self.translate(dx, dy)
    }

    /// 四角与四边中点，顺序：LT, RT, RB, LB, T, R, B, L —— 供八向控制点使用。
    ///
    /// 与 [`Handle::ALL`] 一一对应。需要"某个角柄具体是哪一条边"时用
    /// [`Handle::position_on`]，那个能表达出语义。
    pub fn handles(&self) -> [Pos; 8] {
        Handle::ALL.map(|h| h.position_on(self))
    }

    /// 命中测试：返回离 `p` 最近、且在 `tolerance` 之内的控制点。
    ///
    /// `tolerance` 与坐标同一单位（VSS 物理像素）。角点排在边中点前面，
    /// 所以选区很小时优先抓角 —— 这也是用户的本能预期。
    pub fn handle_at(&self, p: Pos, tolerance: f32) -> Option<Handle> {
        Handle::ALL
            .into_iter()
            .find(|h| p.distance(h.position_on(self)) <= tolerance)
    }

    /// 把 `handle` 控制的边拖到 `to` 所在的位置，返回新矩形。
    ///
    /// - 角点同时控制两条边，边中点只控制一条。
    /// - **不允许翻转**：被拖的边最多推到"对面那条边 ∓ `min_size`"处，
    ///   因此宽高恒 `>= min_size`，`min <= max` 的不变式始终成立。
    ///   这比"先允许翻转再归一化"更符合直觉 —— 后者会让选区在拖过头时
    ///   突然跳到对面，看起来像 bug。
    pub fn resize(&self, handle: Handle, to: Pos, min_size: f32) -> Rect {
        let (mut min, mut max) = (self.min, self.max);

        if handle.moves_left() {
            min.x = to.x;
        } else if handle.moves_right() {
            max.x = to.x;
        }
        if handle.moves_top() {
            min.y = to.y;
        } else if handle.moves_bottom() {
            max.y = to.y;
        }

        if handle.moves_left() {
            min.x = min.x.min(max.x - min_size);
        } else if handle.moves_right() {
            max.x = max.x.max(min.x + min_size);
        }
        if handle.moves_top() {
            min.y = min.y.min(max.y - min_size);
        } else if handle.moves_bottom() {
            max.y = max.y.max(min.y + min_size);
        }

        Rect { min, max }
    }
}

/// 选区八个控制点（角柄）的边长，单位是 **VSS 物理像素**。
///
/// 这个常量同时被**绘制**和**命中测试**读取，从根上排除"看得见的角柄点不中"
/// 或"点得中的位置没有角柄"这类对不上的问题。
///
/// 10px 是权衡后的值：再小在 4K 屏上不好瞄，再大会把选区的可平移区域挤得太窄。
pub const HANDLE_SIZE: f32 = 10.0;

/// 角柄命中半径在角柄本体之外额外放宽的量（物理像素）。
///
/// 角柄本身只有 [`HANDLE_SIZE`] 见方（半边长 5px），对鼠标来说偏小，
/// 因此命中半径取 `HANDLE_SIZE / 2 + HANDLE_HIT_SLOP` = 10px，
/// 也就是每个角柄有 **20px 见方**的抓取范围。
pub const HANDLE_HIT_SLOP: f32 = 5.0;

/// 角柄的命中半径（物理像素）= 角柄半边长 + 外扩容差。
///
/// 直接用它去调 [`Rect::handle_at`]，不要在调用处自己拼加法 ——
/// 那正是"绘制用一套常量、命中用另一套"的温床。
pub const HANDLE_HIT_RADIUS: f32 = HANDLE_SIZE * 0.5 + HANDLE_HIT_SLOP;

/// 选区八个控制点。
///
/// 排序刻意与 [`Rect::handles`] 对齐（四个角在前，四条边在后），
/// 这样命中测试的优先级是"先角后边"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    LeftTop,
    RightTop,
    RightBottom,
    LeftBottom,
    Top,
    Right,
    Bottom,
    Left,
}

impl Handle {
    /// 全部控制点，顺序与 [`Rect::handles`] 一致。
    pub const ALL: [Handle; 8] = [
        Handle::LeftTop,
        Handle::RightTop,
        Handle::RightBottom,
        Handle::LeftBottom,
        Handle::Top,
        Handle::Right,
        Handle::Bottom,
        Handle::Left,
    ];

    /// 是否拖动左边界（`min.x`）。
    pub const fn moves_left(self) -> bool {
        matches!(self, Handle::Left | Handle::LeftTop | Handle::LeftBottom)
    }

    /// 是否拖动右边界（`max.x`）。
    pub const fn moves_right(self) -> bool {
        matches!(self, Handle::Right | Handle::RightTop | Handle::RightBottom)
    }

    /// 是否拖动上边界（`min.y`）。
    pub const fn moves_top(self) -> bool {
        matches!(self, Handle::Top | Handle::LeftTop | Handle::RightTop)
    }

    /// 是否拖动下边界（`max.y`）。
    pub const fn moves_bottom(self) -> bool {
        matches!(
            self,
            Handle::Bottom | Handle::LeftBottom | Handle::RightBottom
        )
    }

    /// 该控制点在 `r` 上的位置。
    pub fn position_on(self, r: &Rect) -> Pos {
        let c = r.center();
        match self {
            Handle::LeftTop => r.min,
            Handle::RightTop => Pos::new(r.max.x, r.min.y),
            Handle::RightBottom => r.max,
            Handle::LeftBottom => Pos::new(r.min.x, r.max.y),
            Handle::Top => Pos::new(c.x, r.min.y),
            Handle::Right => Pos::new(r.max.x, c.y),
            Handle::Bottom => Pos::new(c.x, r.max.y),
            Handle::Left => Pos::new(r.min.x, c.y),
        }
    }
}

/// 点到线段的最短距离 —— 命中测试的核心工具。
///
/// 线段退化成一点时等价于点距。
pub fn distance_point_to_segment(p: Pos, a: Pos, b: Pos) -> f32 {
    let ab = b - a;
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq <= f32::EPSILON {
        return p.distance(a);
    }
    let ap = p - a;
    // 投影参数并钳制到 [0,1]，保证算的是线段而不是无限长直线
    let t = ((ap.x * ab.x + ap.y * ab.y) / len_sq).clamp(0.0, 1.0);
    let proj = Pos::new(a.x + ab.x * t, a.y + ab.y * t);
    p.distance(proj)
}

/// 点到折线（多段线）的最短距离。
pub fn distance_point_to_polyline(p: Pos, points: &[Pos]) -> f32 {
    match points.len() {
        0 => f32::INFINITY,
        1 => p.distance(points[0]),
        _ => points
            .windows(2)
            .map(|w| distance_point_to_segment(p, w[0], w[1]))
            .fold(f32::INFINITY, f32::min),
    }
}

/// 二维仿射变换的便捷封装。
///
/// 用于两处：
/// 1. VSS 物理像素 → 某台显示器的 egui 逻辑坐标
/// 2. 逻辑坐标 → 物理像素（导出时）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub scale: f32,
    pub offset: Pos,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        scale: 1.0,
        offset: Pos::ZERO,
    };

    pub const fn new(scale: f32, offset: Pos) -> Self {
        Self { scale, offset }
    }

    /// `p * scale + offset`
    pub fn apply(&self, p: Pos) -> Pos {
        Pos::new(
            p.x * self.scale + self.offset.x,
            p.y * self.scale + self.offset.y,
        )
    }

    /// 逆变换：`(p - offset) / scale`。scale 为 0 时返回原点（防御性处理）。
    pub fn invert(&self, p: Pos) -> Pos {
        if self.scale.abs() <= f32::EPSILON {
            return Pos::ZERO;
        }
        Pos::new(
            (p.x - self.offset.x) / self.scale,
            (p.y - self.offset.y) / self.scale,
        )
    }

    pub fn apply_rect(&self, r: &Rect) -> Rect {
        Rect::from_points(self.apply(r.min), self.apply(r.max))
    }

    /// 组合：先用 `self`，再用 `other`。
    pub fn then(&self, other: &Transform) -> Transform {
        Transform {
            scale: self.scale * other.scale,
            offset: Pos::new(
                self.offset.x * other.scale + other.offset.x,
                self.offset.y * other.scale + other.offset.y,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_points_normalizes() {
        // 从右下往左上拖，结果必须与从左上往右下拖一致
        let a = Rect::from_points(Pos::new(100.0, 80.0), Pos::new(10.0, 20.0));
        let b = Rect::from_points(Pos::new(10.0, 20.0), Pos::new(100.0, 80.0));
        assert_eq!(a, b);
        assert_eq!(a.min, Pos::new(10.0, 20.0));
        assert_eq!(a.size(), (90.0, 60.0));
    }

    #[test]
    fn rect_intersect_and_union() {
        let a = Rect::new(0.0, 0.0, 100.0, 100.0);
        let b = Rect::new(50.0, 50.0, 100.0, 100.0);
        assert_eq!(a.intersect(&b), Some(Rect::new(50.0, 50.0, 50.0, 50.0)));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 150.0, 150.0));

        let far = Rect::new(500.0, 500.0, 10.0, 10.0);
        assert_eq!(a.intersect(&far), None);
    }

    #[test]
    fn clamp_within_pulls_rect_inside() {
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        // 完全在右下方之外 → 应被推回
        let r = Rect::new(2000.0, 1200.0, 100.0, 50.0);
        let c = r.clamp_within(&bounds);
        assert_eq!(c.max.x, 1920.0);
        assert_eq!(c.max.y, 1080.0);
        assert_eq!(c.width(), 100.0);

        // 已在内部 → 原样返回
        let inside = Rect::new(10.0, 10.0, 100.0, 50.0);
        assert_eq!(inside.clamp_within(&bounds), inside);
    }

    #[test]
    fn point_to_segment_distance() {
        let a = Pos::new(0.0, 0.0);
        let b = Pos::new(100.0, 0.0);
        // 垂足落在线段内
        assert!((distance_point_to_segment(Pos::new(50.0, 10.0), a, b) - 10.0).abs() < 1e-4);
        // 垂足落在线段外 → 退化为端点距离
        assert!((distance_point_to_segment(Pos::new(-30.0, 0.0), a, b) - 30.0).abs() < 1e-4);
        // 退化线段
        assert!((distance_point_to_segment(Pos::new(3.0, 4.0), a, a) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn polyline_distance_takes_minimum() {
        let pts = [Pos::new(0.0, 0.0), Pos::new(10.0, 0.0), Pos::new(20.0, 0.0)];
        assert!((distance_point_to_polyline(Pos::new(15.0, 2.0), &pts) - 2.0).abs() < 1e-4);
        assert_eq!(distance_point_to_polyline(Pos::ZERO, &[]), f32::INFINITY);
    }

    #[test]
    fn transform_roundtrip() {
        // 模拟：VSS 物理像素 → 2 倍缩放屏的逻辑坐标
        let t = Transform::new(0.5, Pos::new(-100.0, -50.0));
        let p = Pos::new(300.0, 250.0);
        let q = t.apply(p);
        assert_eq!(q, Pos::new(50.0, 75.0));
        let back = t.invert(q);
        assert!((back.x - p.x).abs() < 1e-3 && (back.y - p.y).abs() < 1e-3);
    }

    #[test]
    fn rotation_is_reversible() {
        let p = Pos::new(10.0, 0.0);
        let pivot = Pos::new(0.0, 0.0);
        let r = p.rotate_around(pivot, std::f32::consts::FRAC_PI_2);
        assert!(r.x.abs() < 1e-4 && (r.y - 10.0).abs() < 1e-4);
    }

    // ------------------------------------------------------------ 控制点缩放

    #[test]
    fn handles_order_matches_handle_all() {
        // 这条不变式是命中测试优先级的前提：角在前、边在后
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(r.handles(), Handle::ALL.map(|h| h.position_on(&r)));
        assert_eq!(
            r.handles(),
            [
                Pos::new(0.0, 0.0),    // LT
                Pos::new(100.0, 0.0),  // RT
                Pos::new(100.0, 50.0), // RB
                Pos::new(0.0, 50.0),   // LB
                Pos::new(50.0, 0.0),   // T
                Pos::new(100.0, 25.0), // R
                Pos::new(50.0, 50.0),  // B
                Pos::new(0.0, 25.0),   // L
            ]
        );
    }

    #[test]
    fn corner_handle_moves_both_edges() {
        let r = Rect::new(100.0, 100.0, 200.0, 200.0);
        let out = r.resize(Handle::LeftTop, Pos::new(120.0, 130.0), 1.0);
        assert_eq!(out.min, Pos::new(120.0, 130.0));
        assert_eq!(out.max, Pos::new(300.0, 300.0), "右下角不应被碰到");
    }

    #[test]
    fn edge_handle_moves_only_its_own_edge() {
        let r = Rect::new(100.0, 100.0, 200.0, 200.0);
        let out = r.resize(Handle::Right, Pos::new(400.0, 999.0), 1.0);
        // y 完全不受影响 —— 这是"只有一条边动"的关键
        assert_eq!(out.min, Pos::new(100.0, 100.0));
        assert_eq!(out.max, Pos::new(400.0, 300.0));
    }

    #[test]
    fn resize_never_flips_past_the_opposite_edge() {
        let r = Rect::new(100.0, 100.0, 200.0, 200.0);
        // 把左边界一路拖到右边界的右侧
        let out = r.resize(Handle::Left, Pos::new(9999.0, 0.0), 10.0);
        assert_eq!(out.width(), 10.0, "应停在最小尺寸，而不是翻转");
        assert_eq!(out.max.x, 300.0);

        let out = r.resize(Handle::Right, Pos::new(-9999.0, 0.0), 10.0);
        assert_eq!(out.width(), 10.0);
        assert_eq!(out.min.x, 100.0);
    }

    #[test]
    fn handle_at_prefers_corner_over_edge_midpoint() {
        // 退化矩形：四角与四边中点全部重合，应当仍能稳定判出"角"
        let tiny = Rect::new(10.0, 10.0, 0.0, 0.0);
        let h = tiny.handle_at(Pos::new(10.0, 10.0), 5.0);
        assert!(
            matches!(
                h,
                Some(Handle::LeftTop)
                    | Some(Handle::RightTop)
                    | Some(Handle::RightBottom)
                    | Some(Handle::LeftBottom)
            ),
            "应命中角点，实际 {h:?}"
        );
    }

    #[test]
    fn handle_at_is_none_when_far_away() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            r.handle_at(Pos::new(50.0, 50.0), 5.0),
            None,
            "中心不该命中角柄"
        );
        assert_eq!(r.handle_at(Pos::new(500.0, 500.0), 5.0), None);
        // 容差足够大时能命中最近的角
        assert_eq!(r.handle_at(Pos::new(3.0, 3.0), 10.0), Some(Handle::LeftTop));
    }
}
