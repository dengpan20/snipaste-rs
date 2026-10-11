//! 笔迹简化与平滑。
//!
//! 自由画笔每秒可能产生上百个采样点（高刷鼠标 + 高分屏更甚）。
//! 一支 10 秒的长笔迹轻易就有上千个点，直接交给三角化器会造成两个问题：
//!
//! 1. **每帧成本高**：egui 是 immediate mode，每帧都要重新三角化一遍
//! 2. **文件体积大**：若将来支持导出可编辑工程，原始点会撑大文件
//!
//! 两个函数各司其职：
//!
//! - [`douglas_peucker`]：抽稀，去掉视觉上冗余的点（通常能减 60%~80%）
//! - [`catmull_rom_to_bezier`]：把折线平滑成贝塞尔曲线段，消除折角

use crate::geometry::{distance_point_to_segment, Pos};

/// 一段三次贝塞尔曲线。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicSeg {
    pub p0: Pos,
    pub c1: Pos,
    pub c2: Pos,
    pub p3: Pos,
}

impl CubicSeg {
    /// 按参数 `t ∈ [0, 1]` 求值。`t = 0` 返回 `p0`，`t = 1` 返回 `p3`。
    ///
    /// 用于曲线图元的命中测试与马赛克路径采样 —— 与渲染共用同一份公式，
    /// 保证"点得中"和"看得见"是同一个形状。
    pub fn eval(&self, t: f32) -> Pos {
        let u = 1.0 - t;
        let (uu, tt) = (u * u, t * t);
        Pos::new(
            uu * u * self.p0.x
                + 3.0 * uu * t * self.c1.x
                + 3.0 * u * tt * self.c2.x
                + tt * t * self.p3.x,
            uu * u * self.p0.y
                + 3.0 * uu * t * self.c1.y
                + 3.0 * u * tt * self.c2.y
                + tt * t * self.p3.y,
        )
    }

    /// 把曲线离散成折线（供快速命中测试或拼多边形用）。
    pub fn flatten(&self, segments: usize) -> Vec<Pos> {
        let n = segments.max(2);
        (0..=n).map(|i| self.eval(i as f32 / n as f32)).collect()
    }
}

/// Douglas-Peucker 抽稀。
///
/// 保留能代表形状的"拐点"，丢弃在拟合误差 `epsilon` 之内的中间点。
///
/// - `epsilon` 单位是 VSS 物理像素。0.5~1.0 是常用值：肉眼几乎无差别，
///   但点数通常能减少 60%~80%。
/// - 采用**显式栈**迭代而非递归。长笔迹可能有上万点，递归会因为
///   最坏情况（点恰好按距离递增排列）导致栈溢出。
///
/// 首尾点永远保留，因此不会改变笔迹的起止位置。
pub fn douglas_peucker(points: &[Pos], epsilon: f32) -> Vec<Pos> {
    if points.len() < 3 || epsilon <= 0.0 {
        return points.to_vec();
    }

    let last = points.len() - 1;
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[last] = true;

    let mut stack: Vec<(usize, usize)> = vec![(0, last)];
    while let Some((first, end)) = stack.pop() {
        // 相邻两点之间没有可抽的点
        if end <= first + 1 {
            continue;
        }

        // 找出离首尾连线最远的点
        let mut worst_dist = 0.0f32;
        let mut worst_idx = first;
        for i in (first + 1)..end {
            let d = distance_point_to_segment(points[i], points[first], points[end]);
            if d > worst_dist {
                worst_dist = d;
                worst_idx = i;
            }
        }

        // 超出容差 → 该点是拐点，保留并继续细分两侧
        if worst_dist > epsilon {
            keep[worst_idx] = true;
            stack.push((first, worst_idx));
            stack.push((worst_idx, end));
        }
    }

    points
        .iter()
        .zip(keep.iter())
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// Catmull-Rom 样条转三次贝塞尔曲线段。
///
/// 逐段独立地理解 Catmull-Rom 会产生折角；转成贝塞尔交给渲染器后，
/// 曲线在连接处切线连续（C¹），看起来才是一条顺滑的笔迹。
///
/// ## 控制点推导
///
/// 对相邻两点 `p1 → p2`，取前后各一点 `p0` / `p3` 决定切线：
///
/// ```text
/// c1 = p1 + (p2 - p0) / 6 * tension
/// c2 = p2 - (p3 - p1) / 6 * tension
/// ```
///
/// `tension = 1.0` 即标准 Catmull-Rom。端点处通过**复制端点**来构造虚拟的
/// `p0` / `p3`，这样首尾段不会出现异常甩尾。
pub fn catmull_rom_to_bezier(points: &[Pos], tension: f32) -> Vec<CubicSeg> {
    if points.len() < 2 {
        return Vec::new();
    }

    let n = points.len();
    let at = |i: isize| -> Pos {
        // 端点复制：索引越界时钳制到最近的端点
        let idx = i.clamp(0, n as isize - 1) as usize;
        points[idx]
    };

    (0..n - 1)
        .map(|i| {
            let p0 = at(i as isize - 1);
            let p1 = at(i as isize);
            let p2 = at(i as isize + 1);
            let p3 = at(i as isize + 2);

            CubicSeg {
                p0: p1,
                c1: Pos::new(
                    p1.x + (p2.x - p0.x) / 6.0 * tension,
                    p1.y + (p2.y - p0.y) / 6.0 * tension,
                ),
                c2: Pos::new(
                    p2.x - (p3.x - p1.x) / 6.0 * tension,
                    p2.y - (p3.y - p1.y) / 6.0 * tension,
                ),
                p3: p2,
            }
        })
        .collect()
}

/// 采样滤波：在记录笔迹时丢弃与前一点距离过近的点。
///
/// 高刷鼠标 / 高 DPI 屏上，一次轻微移动会产生几十个几乎重合的点。
/// 这一步在**记录阶段**就做掉，比事后抽稀更省内存。
///
/// 返回是否接受了这个点，便于调用方直接串联。
pub fn accept_sample(points: &[Pos], candidate: Pos, min_distance: f32) -> bool {
    match points.last() {
        None => true,
        Some(last) => last.distance(candidate) >= min_distance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dp_keeps_endpoints() {
        let pts: Vec<Pos> = (0..=10).map(|i| Pos::new(i as f32, 0.0)).collect();
        let out = douglas_peucker(&pts, 0.5);
        // 完全共线 → 只剩首尾
        assert_eq!(out, vec![Pos::new(0.0, 0.0), Pos::new(10.0, 0.0)]);
    }

    #[test]
    fn dp_preserves_significant_corner() {
        // 一个明显的直角拐点
        let pts = vec![
            Pos::new(0.0, 0.0),
            Pos::new(50.0, 0.0),
            Pos::new(100.0, 0.0),
            Pos::new(100.0, 100.0),
            Pos::new(100.0, 200.0),
        ];
        let out = douglas_peucker(&pts, 1.0);
        assert!(out.contains(&Pos::new(100.0, 0.0)), "直角拐点必须保留");
        assert_eq!(out.first(), Some(&Pos::new(0.0, 0.0)));
        assert_eq!(out.last(), Some(&Pos::new(100.0, 200.0)));
        assert!(out.len() < pts.len(), "应当有抽稀效果");
    }

    #[test]
    fn dp_epsilon_zero_is_identity() {
        let pts = vec![Pos::new(0.0, 0.0), Pos::new(1.0, 0.5), Pos::new(2.0, 0.0)];
        assert_eq!(douglas_peucker(&pts, 0.0), pts);
    }

    #[test]
    fn dp_short_input_is_identity() {
        let pts = vec![Pos::new(0.0, 0.0), Pos::new(5.0, 5.0)];
        assert_eq!(douglas_peucker(&pts, 1.0), pts);
        assert_eq!(douglas_peucker(&[], 1.0).len(), 0);
    }

    #[test]
    fn dp_handles_large_input_without_stack_overflow() {
        // 一万个点，且刻意构造"距离单调递增"的对抗性输入。
        // 递归实现会在这里爆栈，迭代实现不会。
        let pts: Vec<Pos> = (0..10_000)
            .map(|i| Pos::new(i as f32, (i as f32 * 0.001).sin() * (i as f32) * 0.0001))
            .collect();
        let out = douglas_peucker(&pts, 0.6);
        assert!(out.len() >= 2);
        assert!(out.len() < pts.len());
    }

    #[test]
    fn dp_reduces_point_count_substantially() {
        // 模拟一段圆弧上的密集采样
        let pts: Vec<Pos> = (0..=500)
            .map(|i| {
                let a = i as f32 / 500.0 * std::f32::consts::PI;
                Pos::new(a.cos() * 200.0, a.sin() * 200.0)
            })
            .collect();
        let out = douglas_peucker(&pts, 0.8);
        assert!(
            out.len() < pts.len() / 3,
            "圆弧抽稀后应大幅减少点数, got {} / {}",
            out.len(),
            pts.len()
        );
    }

    #[test]
    fn catmull_rom_segment_count() {
        let pts = vec![
            Pos::new(0.0, 0.0),
            Pos::new(10.0, 10.0),
            Pos::new(20.0, 0.0),
            Pos::new(30.0, 10.0),
        ];
        let segs = catmull_rom_to_bezier(&pts, 1.0);
        assert_eq!(segs.len(), pts.len() - 1);
        // 每段的起点终点必须精确落在原始采样点上
        assert_eq!(segs[0].p0, pts[0]);
        assert_eq!(segs[0].p3, pts[1]);
        assert_eq!(segs[2].p3, pts[3]);
    }

    #[test]
    fn catmull_rom_endpoints_are_clamped() {
        // 首段的 c1 用的是虚拟点 p0 = points[0]，
        // 因此切线只由 p2 - p0 决定，不会异常甩出去
        let pts = vec![Pos::new(0.0, 0.0), Pos::new(10.0, 0.0), Pos::new(20.0, 0.0)];
        let segs = catmull_rom_to_bezier(&pts, 1.0);
        assert_eq!(segs[0].c1, Pos::new(10.0 / 6.0, 0.0));
    }

    #[test]
    fn catmull_rom_degenerate_inputs() {
        assert!(catmull_rom_to_bezier(&[], 1.0).is_empty());
        assert!(catmull_rom_to_bezier(&[Pos::ZERO], 1.0).is_empty());
        let segs = catmull_rom_to_bezier(&[Pos::ZERO, Pos::new(1.0, 1.0)], 1.0);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn cubic_eval_hits_endpoints() {
        let s = CubicSeg {
            p0: Pos::new(0.0, 0.0),
            c1: Pos::new(0.0, 100.0),
            c2: Pos::new(100.0, 100.0),
            p3: Pos::new(100.0, 0.0),
        };
        assert_eq!(s.eval(0.0), s.p0);
        assert_eq!(s.eval(1.0), s.p3);
        // 中点偏移到曲线上方（由控制点决定）
        assert!(s.eval(0.5).y > 50.0);
    }

    #[test]
    fn flatten_produces_requested_resolution() {
        let s = CubicSeg {
            p0: Pos::ZERO,
            c1: Pos::new(0.0, 10.0),
            c2: Pos::new(10.0, 10.0),
            p3: Pos::new(10.0, 0.0),
        };
        assert_eq!(s.flatten(0).len(), 3, "segments 下限被钳制为 2 → 3 个点");
        assert_eq!(s.flatten(16).len(), 17);
    }

    #[test]
    fn accept_sample_filters_dense_points() {
        let mut pts: Vec<Pos> = Vec::new();
        assert!(accept_sample(&pts, Pos::new(0.0, 0.0), 1.5));
        pts.push(Pos::new(0.0, 0.0));

        // 距离过近 → 拒绝
        assert!(!accept_sample(&pts, Pos::new(0.5, 0.0), 1.5));
        // 距离足够 → 接受
        assert!(accept_sample(&pts, Pos::new(2.0, 0.0), 1.5));
    }
}
