//! 标注图元模型。
//!
//! ## 核心设计：只存矢量参数，绝不存位图
//!
//! 一个带 500 个采样点的画笔描边约 4 KB，即使堆满 100 个图元也只占几百 KB。
//! 这直接决定了撤销栈可以无上限保留 —— 每次操作产生一条 `Edit` 记录，而不是
//! 一整张位图快照。若改成快照式撤销，每画一笔就要复制整个选区，内存会随操作
//! 次数线性膨胀，与"内存小"的硬性要求正面对撞。

use crate::geometry::{distance_point_to_polyline, Pos, Rect};
use crate::style::Style;

/// 箭头形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowKind {
    /// 直线箭头（最常用）。
    Straight,
    /// 折线箭头：先沿主轴走，再拐向目标点。
    Elbow,
    /// 曲线箭头：以二次贝塞尔弯曲。
    Curve,
}

/// 自由笔迹的两种语义。几何完全相同，差异全在样式与渲染细节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// 画笔：细、实色、圆头圆角。
    Pen,
    /// 记号笔：粗、半透明、平头斜接（模拟马克笔）。
    Marker,
}

/// 马赛克的作用区域。
#[derive(Debug, Clone, PartialEq)]
pub enum MosaicRegion {
    /// 矩形区域（框选马赛克）。
    Rect(Rect),
    /// 连续笔刷的中心线，半径由图元样式的 stroke_width / 2 决定。
    Path(Vec<Pos>),
}

impl MosaicRegion {
    pub fn bounds(&self) -> Rect {
        match self {
            MosaicRegion::Rect(r) => *r,
            MosaicRegion::Path(pts) => bounding_of(pts),
        }
    }
}

/// 一个标注图元。
///
/// 所有坐标都是 VSS 物理像素；所有尺寸（线宽、字号、马赛克块）也都是物理像素。
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    Rect {
        rect: Rect,
        style: Style,
    },
    Ellipse {
        /// 外接矩形 —— 椭圆由它唯一确定。
        rect: Rect,
        style: Style,
    },
    Arrow {
        from: Pos,
        to: Pos,
        kind: ArrowKind,
        style: Style,
    },
    Path {
        points: Vec<Pos>,
        kind: PathKind,
        style: Style,
    },
    Mosaic {
        region: MosaicRegion,
        style: Style,
    },
    /// 局部恢复冻结底图；按绘制顺序盖住此前的标注，后续标注仍可画在上面。
    Eraser {
        region: MosaicRegion,
        style: Style,
    },
    Text {
        rect: Rect,
        content: String,
        style: Style,
    },
    Sequence {
        center: Pos,
        number: u32,
        radius: f32,
        style: Style,
    },
}

impl Annotation {
    pub fn style(&self) -> &Style {
        match self {
            Self::Rect { style, .. }
            | Self::Ellipse { style, .. }
            | Self::Arrow { style, .. }
            | Self::Path { style, .. }
            | Self::Mosaic { style, .. }
            | Self::Eraser { style, .. }
            | Self::Text { style, .. }
            | Self::Sequence { style, .. } => style,
        }
    }

    pub fn style_mut(&mut self) -> &mut Style {
        match self {
            Self::Rect { style, .. }
            | Self::Ellipse { style, .. }
            | Self::Arrow { style, .. }
            | Self::Path { style, .. }
            | Self::Mosaic { style, .. }
            | Self::Eraser { style, .. }
            | Self::Text { style, .. }
            | Self::Sequence { style, .. } => style,
        }
    }

    /// 是否为"可填充"图元（矩形 / 椭圆 / 文字背景）。
    pub fn supports_fill(&self) -> bool {
        matches!(self, Self::Rect { .. } | Self::Ellipse { .. })
    }

    /// 是否为"可调笔宽"图元。
    pub fn supports_stroke_width(&self) -> bool {
        !matches!(
            self,
            Self::Text { .. } | Self::Mosaic { .. } | Self::Sequence { .. }
        )
    }

    /// 图元的外接矩形（VSS 物理像素）。
    ///
    /// 会额外把描边宽度的一半算进去，否则细线的包围盒会漏掉边缘，
    /// 导致"框选导出"时裁掉半个描边。
    pub fn bounds(&self) -> Rect {
        if let Self::Sequence { center, radius, .. } = self {
            return Rect::new(
                center.x - radius,
                center.y - radius,
                radius * 2.0,
                radius * 2.0,
            );
        }
        if let Self::Mosaic {
            region: MosaicRegion::Rect(r),
            ..
        }
        | Self::Eraser {
            region: MosaicRegion::Rect(r),
            ..
        } = self
        {
            return *r;
        }
        let raw = match self {
            Self::Rect { rect, .. } | Self::Ellipse { rect, .. } => *rect,
            Self::Arrow { from, to, kind, .. } => bounding_of(&arrow_polyline(*from, *to, *kind)),
            Self::Path { points, .. } => bounding_of(points),
            Self::Mosaic { region, .. } | Self::Eraser { region, .. } => region.bounds(),
            Self::Text { rect, .. } => *rect,
            Self::Sequence { .. } => unreachable!(),
        };
        raw.expand(self.style().stroke_width * 0.5)
    }

    /// 平移（拖动已有图元）。
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Self::Rect { rect, .. } | Self::Ellipse { rect, .. } => {
                *rect = rect.translate(dx, dy);
            }
            Self::Arrow { from, to, .. } => {
                *from = Pos::new(from.x + dx, from.y + dy);
                *to = Pos::new(to.x + dx, to.y + dy);
            }
            Self::Path { points, .. } => {
                for p in points.iter_mut() {
                    *p = Pos::new(p.x + dx, p.y + dy);
                }
            }
            Self::Mosaic { region, .. } | Self::Eraser { region, .. } => match region {
                MosaicRegion::Rect(r) => *r = r.translate(dx, dy),
                MosaicRegion::Path(pts) => {
                    for p in pts.iter_mut() {
                        *p = Pos::new(p.x + dx, p.y + dy);
                    }
                }
            },
            Self::Text { rect, .. } => *rect = rect.translate(dx, dy),
            Self::Sequence { center, .. } => *center = *center + Pos::new(dx, dy),
        }
    }

    /// 命中测试：VSS 点 `p` 是否落在该图元上（用于橡皮擦与拖动）。
    ///
    /// `extra` 是额外的容差（物理像素），通常传 0，由图元自己的
    /// [`Style::hit_tolerance`] 提供基础容差 —— 保证 1px 的细线也点得中。
    pub fn hit_test(&self, p: Pos, extra: f32) -> bool {
        let tol = self.style().hit_tolerance() + extra;
        match self {
            Self::Rect { rect, style } => {
                if style.fill.is_some_and(|f| !f.is_transparent()) {
                    rect.contains(p)
                } else {
                    within_rect_border(p, rect, tol)
                }
            }
            Self::Ellipse { rect, style } => {
                let c = rect.center();
                let (rx, ry) = (rect.width() * 0.5, rect.height() * 0.5);
                if rx <= f32::EPSILON || ry <= f32::EPSILON {
                    return false;
                }
                let dx = (p.x - c.x) / rx;
                let dy = (p.y - c.y) / ry;
                let v = dx * dx + dy * dy;
                if style.fill.is_some_and(|f| !f.is_transparent()) {
                    v <= 1.0
                } else {
                    // 空心椭圆：用采样折线近似轮廓，比解椭圆方程稳健得多
                    distance_point_to_polyline(p, &ellipse_outline(rect, 72)) <= tol
                }
            }
            Self::Arrow { from, to, kind, .. } => {
                let shaft = arrow_polyline(*from, *to, *kind);
                distance_point_to_polyline(p, &shaft) <= tol
            }
            Self::Path { points, .. } => distance_point_to_polyline(p, points) <= tol,
            Self::Mosaic { region, style } | Self::Eraser { region, style } => match region {
                MosaicRegion::Rect(r) => r.contains(p),
                MosaicRegion::Path(pts) => {
                    distance_point_to_polyline(p, pts) <= style.stroke_width * 0.5 + extra
                }
            },
            Self::Text { rect, .. } => rect.contains(p),
            Self::Sequence { center, radius, .. } => center.distance(p) <= radius + extra,
        }
    }

    /// 用于日志与调试面板的短名称。
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Rect { .. } => "矩形",
            Self::Ellipse { .. } => "椭圆",
            Self::Arrow { kind, .. } => match kind {
                ArrowKind::Straight => "箭头",
                ArrowKind::Elbow => "折线箭头",
                ArrowKind::Curve => "曲线箭头",
            },
            Self::Path { kind, .. } => match kind {
                PathKind::Pen => "画笔",
                PathKind::Marker => "记号笔",
            },
            Self::Mosaic { .. } => "马赛克",
            Self::Eraser { .. } => "橡皮擦",
            Self::Text { .. } => "文字",
            Self::Sequence { .. } => "序号",
        }
    }
}

/// 箭头箭杆的折线近似。
///
/// **渲染与命中测试共用此函数**，保证"看到的形状"与"点得中的区域"永远一致。
/// 曲线箭头离散成 24 段，视觉上已完全平滑。
pub fn arrow_polyline(from: Pos, to: Pos, kind: ArrowKind) -> Vec<Pos> {
    match kind {
        ArrowKind::Straight => vec![from, to],
        ArrowKind::Elbow => {
            let d = to - from;
            if d.x.abs() >= d.y.abs() {
                // 主轴为水平：先水平走到目标的 x，再竖直
                vec![from, Pos::new(to.x, from.y), to]
            } else {
                vec![from, Pos::new(from.x, to.y), to]
            }
        }
        ArrowKind::Curve => {
            // 控制点取中点沿法线偏移 25% 箭头长度，形成柔和的弧。
            // 偏移量按长度成比例，因此长短箭头看起来是"同一种弧度风格"。
            let mid = from.lerp(to, 0.5);
            let d = to - from;
            let arrow_len = d.length();
            let ctrl = if arrow_len <= f32::EPSILON {
                mid
            } else {
                // 把 d 旋转 90° 得到法线（长度不变），归一化后再乘 25% 箭长
                let unit_normal = Pos::new(-d.y / arrow_len, d.x / arrow_len);
                let k = arrow_len * 0.25;
                Pos::new(mid.x + unit_normal.x * k, mid.y + unit_normal.y * k)
            };
            const SEGMENTS: usize = 24;
            (0..=SEGMENTS)
                .map(|i| {
                    let t = i as f32 / SEGMENTS as f32;
                    let u = 1.0 - t;
                    // 二次贝塞尔
                    Pos::new(
                        u * u * from.x + 2.0 * u * t * ctrl.x + t * t * to.x,
                        u * u * from.y + 2.0 * u * t * ctrl.y + t * t * to.y,
                    )
                })
                .collect()
        }
    }
}

/// 生成椭圆轮廓的采样点（空心椭圆的命中测试与描边共用）。
pub fn ellipse_outline(rect: &Rect, segments: usize) -> Vec<Pos> {
    let c = rect.center();
    let (rx, ry) = (rect.width() * 0.5, rect.height() * 0.5);
    let n = segments.max(8);
    (0..=n)
        .map(|i| {
            let a = std::f32::consts::TAU * (i as f32) / (n as f32);
            Pos::new(c.x + rx * a.cos(), c.y + ry * a.sin())
        })
        .collect()
}

/// 一组点的外接矩形。空集合返回零矩形。
pub fn bounding_of(points: &[Pos]) -> Rect {
    if points.is_empty() {
        return Rect {
            min: Pos::ZERO,
            max: Pos::ZERO,
        };
    }
    let mut min = points[0];
    let mut max = points[0];
    for p in points {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    Rect { min, max }
}

/// 点是否在矩形边框（而非内部）的容差范围内。
fn within_rect_border(p: Pos, r: &Rect, tol: f32) -> bool {
    let corners = [
        r.min,
        Pos::new(r.max.x, r.min.y),
        r.max,
        Pos::new(r.min.x, r.max.y),
        r.min, // 闭合
    ];
    distance_point_to_polyline(p, &corners) <= tol
}

/// 折线总长度（用于渲染时的抗锯齿补偿与调试）。
pub fn polyline_length(points: &[Pos]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Color, Style};

    fn style() -> Style {
        Style {
            stroke: Color::rgb(255, 0, 0),
            stroke_width: 4.0,
            ..Style::default()
        }
    }

    #[test]
    fn rect_hit_test_border_only_when_hollow() {
        let a = Annotation::Rect {
            rect: Rect::new(100.0, 100.0, 200.0, 100.0),
            style: style(),
        };
        // 边框上 → 命中
        assert!(a.hit_test(Pos::new(100.0, 150.0), 0.0));
        assert!(a.hit_test(Pos::new(200.0, 100.0), 0.0));
        // 正中心 → 空心矩形不该命中
        assert!(!a.hit_test(Pos::new(200.0, 150.0), 0.0));
    }

    #[test]
    fn rect_hit_test_interior_when_filled() {
        let mut s = style();
        s.fill = Some(Color::rgb(0, 0, 255));
        let a = Annotation::Rect {
            rect: Rect::new(100.0, 100.0, 200.0, 100.0),
            style: s,
        };
        assert!(a.hit_test(Pos::new(200.0, 150.0), 0.0));
        // 外部仍不命中
        assert!(!a.hit_test(Pos::new(400.0, 150.0), 0.0));
    }

    #[test]
    fn transparent_fill_does_not_count_as_filled() {
        let mut s = style();
        s.fill = Some(Color::rgba(0, 0, 255, 0));
        let a = Annotation::Rect {
            rect: Rect::new(100.0, 100.0, 200.0, 100.0),
            style: s,
        };
        // 完全透明的填充等同空心
        assert!(!a.hit_test(Pos::new(200.0, 150.0), 0.0));
    }

    #[test]
    fn ellipse_hit_test() {
        let a = Annotation::Ellipse {
            rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            style: style(),
        };
        // 右侧顶点在轮廓上
        assert!(a.hit_test(Pos::new(200.0, 50.0), 0.0));
        // 中心不命中（空心）
        assert!(!a.hit_test(Pos::new(100.0, 50.0), 0.0));
        // 外接矩形的角落不命中 —— 这正是椭圆与矩形的区别
        assert!(!a.hit_test(Pos::new(2.0, 2.0), 0.0));
    }

    #[test]
    fn degenerate_ellipse_never_panics() {
        let a = Annotation::Ellipse {
            rect: Rect::new(10.0, 10.0, 0.0, 0.0),
            style: style(),
        };
        assert!(!a.hit_test(Pos::new(10.0, 10.0), 0.0));
    }

    #[test]
    fn arrow_hit_test_along_shaft() {
        let a = Annotation::Arrow {
            from: Pos::new(0.0, 0.0),
            to: Pos::new(200.0, 0.0),
            kind: ArrowKind::Straight,
            style: style(),
        };
        assert!(a.hit_test(Pos::new(100.0, 1.0), 0.0));
        assert!(!a.hit_test(Pos::new(100.0, 60.0), 0.0));
    }

    #[test]
    fn elbow_arrow_goes_through_corner() {
        let pts = arrow_polyline(Pos::new(0.0, 0.0), Pos::new(100.0, 80.0), ArrowKind::Elbow);
        assert_eq!(pts.len(), 3);
        // 主轴为 x（|dx| > |dy|）→ 先水平
        assert_eq!(pts[1], Pos::new(100.0, 0.0));

        let pts2 = arrow_polyline(Pos::new(0.0, 0.0), Pos::new(50.0, 200.0), ArrowKind::Elbow);
        // 主轴为 y → 先竖直
        assert_eq!(pts2[1], Pos::new(0.0, 200.0));
    }

    #[test]
    fn curve_arrow_is_sampled_and_bends() {
        let pts = arrow_polyline(Pos::new(0.0, 0.0), Pos::new(100.0, 0.0), ArrowKind::Curve);
        assert_eq!(pts.len(), 25);
        // 端点必须精确落在起止点上
        assert_eq!(pts[0], Pos::new(0.0, 0.0));
        assert_eq!(*pts.last().unwrap(), Pos::new(100.0, 0.0));
        // 中间应当偏离直线（真的弯了）
        let mid = pts[12];
        assert!(mid.y.abs() > 1.0, "曲线箭头中点应偏离直线, got {mid:?}");
    }

    #[test]
    fn bounds_include_stroke_width() {
        let a = Annotation::Rect {
            rect: Rect::new(100.0, 100.0, 200.0, 100.0),
            style: style(), // stroke_width = 4.0
        };
        let b = a.bounds();
        // 四周各外扩一半线宽
        assert_eq!(b.min, Pos::new(98.0, 98.0));
        assert_eq!(b.max, Pos::new(302.0, 202.0));
    }

    #[test]
    fn translate_moves_all_variants() {
        let mut cases = [
            Annotation::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                style: style(),
            },
            Annotation::Ellipse {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                style: style(),
            },
            Annotation::Arrow {
                from: Pos::ZERO,
                to: Pos::new(10.0, 10.0),
                kind: ArrowKind::Straight,
                style: style(),
            },
            Annotation::Path {
                points: vec![Pos::ZERO, Pos::new(10.0, 10.0)],
                kind: PathKind::Pen,
                style: style(),
            },
            Annotation::Mosaic {
                region: MosaicRegion::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
                style: style(),
            },
            Annotation::Eraser {
                region: MosaicRegion::Path(vec![Pos::ZERO, Pos::new(10.0, 10.0)]),
                style: style(),
            },
            Annotation::Text {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                content: "hi".into(),
                style: style(),
            },
        ];
        for a in cases.iter_mut() {
            let before = a.bounds();
            a.translate(5.0, -3.0);
            let after = a.bounds();
            assert!(
                (after.min.x - before.min.x - 5.0).abs() < 1e-4,
                "{} 未正确平移",
                a.kind_name()
            );
            assert!((after.min.y - before.min.y + 3.0).abs() < 1e-4);
        }
    }

    #[test]
    fn pen_path_hit_test_uses_polyline() {
        let a = Annotation::Path {
            points: vec![
                Pos::new(0.0, 0.0),
                Pos::new(50.0, 0.0),
                Pos::new(50.0, 50.0),
            ],
            kind: PathKind::Pen,
            style: style(),
        };
        assert!(a.hit_test(Pos::new(25.0, 2.0), 0.0));
        assert!(a.hit_test(Pos::new(52.0, 30.0), 0.0));
        assert!(!a.hit_test(Pos::new(0.0, 40.0), 0.0));
    }

    #[test]
    fn bounding_of_empty_is_zero_rect() {
        let r = bounding_of(&[]);
        assert!(r.is_empty());
    }

    #[test]
    fn polyline_length_sums_segments() {
        let pts = [Pos::new(0.0, 0.0), Pos::new(3.0, 4.0), Pos::new(3.0, 14.0)];
        assert!((polyline_length(&pts) - 15.0).abs() < 1e-4);
    }
}
