//! VSS（Virtual Screen Space）虚拟屏坐标系 —— 多显示器 / 混合 DPI 的核心抽象。
//!
//! ## 为什么需要它
//!
//! 多显示器 + 混合 DPI 下，有两套互相冲突的坐标：
//!
//! - **物理像素**：屏幕的真实像素。`4K 屏 = 3840×2160`
//! - **逻辑点 / DIP**：操作系统缩放后的坐标。`4K@200% = 1920×1080 逻辑点`
//!
//! 截图的位图是物理像素，而窗口 / 鼠标事件是逻辑坐标。如果在一个
//! 跨显示器的窗口里混用两者，一定会出现"截图区域偏移"或"图像被二次缩放变模糊"。
//!
//! ## 约定
//!
//! VSS 把所有显示器按**物理像素**拼成一张大画布，原点取所有显示器并集的左上角。
//!
//! ```text
//! 显示器 0：1920×1080 @ 100%         显示器 1：3840×2160 @ 200%
//!
//! VSS 原点 (0,0)
//!   ┌──────────────────┬────────────────────────────────────────┐
//!   │   monitor 0      │   monitor 1                            │
//!   │   物理 1920×1080 │   物理 3840×2160                        │
//!   │   scale = 1.0    │   scale = 2.0                          │
//!   └──────────────────┴────────────────────────────────────────┘
//!                                              VSS 右下角 (5760, 2160)
//! ```
//!
//! 标注图元、选区、鼠标位置一律在 VSS 中表达；只在绘制到某一块屏时，
//! 才用 [`VssToScreen`] 转换成该屏的逻辑坐标。

use crate::geometry::{Pos, Rect, Transform};

/// 单台显示器的信息。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    /// 平台相关的显示器标识（Windows 上是 `HMONITOR` 值，macOS 上是 `CGDirectDisplayID`）。
    pub id: u64,
    /// 显示器名，仅用于日志与调试面板。
    pub name: String,
    /// 该显示器在 VSS 中占据的矩形（物理像素）。
    pub vss_rect: Rect,
    /// 缩放倍率。100% → 1.0；macOS Retina 通常为 2.0。
    pub scale_factor: f64,
    /// 是否为主显示器。工具栏首次出现的位置会优先参考主屏。
    pub is_primary: bool,
}

impl MonitorInfo {
    /// 该屏的物理尺寸。
    pub fn physical_size(&self) -> (u32, u32) {
        (
            self.vss_rect.width().round().max(0.0) as u32,
            self.vss_rect.height().round().max(0.0) as u32,
        )
    }

    /// 该屏的逻辑尺寸（物理像素 ÷ 缩放倍率）。
    pub fn logical_size(&self) -> (f32, f32) {
        let s = self.scale_factor as f32;
        let (w, h) = self.physical_size();
        (w as f32 / s, h as f32 / s)
    }

    /// VSS 坐标 → 该屏的**逻辑坐标**（egui / winit 使用的坐标）。
    pub fn transform(&self) -> Transform {
        let s = self.scale_factor as f32;
        // 先减去本屏在 VSS 中的原点，再按缩放倍率折算成逻辑点
        Transform::new(
            1.0 / s,
            Pos::new(-self.vss_rect.min.x / s, -self.vss_rect.min.y / s),
        )
    }

    /// VSS 坐标 → 该屏的**物理像素**（捕获、导出、行缓存索引使用）。
    pub fn to_local_physical(&self, p: Pos) -> Pos {
        Pos::new(p.x - self.vss_rect.min.x, p.y - self.vss_rect.min.y)
    }
}

/// 全部显示器的 VSS 布局。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VssLayout {
    pub monitors: Vec<MonitorInfo>,
    /// 所有显示器的并集外接矩形。
    pub bounds: Rect,
}

impl VssLayout {
    /// 由显示器列表构造，自动计算并集边界。
    ///
    /// 传入空列表时得到空布局（各 API 均能安全处理，不会 panic）。
    pub fn new(monitors: Vec<MonitorInfo>) -> Self {
        let bounds = monitors
            .iter()
            .map(|m| m.vss_rect)
            .reduce(|acc, r| acc.union(&r))
            .unwrap_or(Rect {
                min: Pos::ZERO,
                max: Pos::ZERO,
            });
        Self { monitors, bounds }
    }

    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.monitors.len()
    }

    /// 命中测试：VSS 点落在哪块屏上。
    ///
    /// 点在所有屏之外（例如显示器排列有空洞）时返回 `None`。
    pub fn monitor_at(&self, p: Pos) -> Option<&MonitorInfo> {
        self.monitors.iter().find(|m| m.vss_rect.contains(p))
    }

    pub fn primary(&self) -> Option<&MonitorInfo> {
        self.monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| self.monitors.first())
    }

    /// 选区覆盖了哪些显示器。跨屏截图需要按此顺序拼接底图。
    ///
    /// 返回顺序与 `monitors` 顺序一致，保证每次拼接结果稳定。
    pub fn monitors_covering(&self, region: &Rect) -> Vec<&MonitorInfo> {
        self.monitors
            .iter()
            .filter(|m| m.vss_rect.intersect(region).is_some())
            .collect()
    }

    /// 是否存在混合缩放倍率。
    ///
    /// 用于在设置页给出提示，以及决定是否需要走"每屏一个遮罩窗口"的路径
    /// （虽然本方案一律走该路径，但该信息对诊断很有价值）。
    pub fn has_mixed_dpi(&self) -> bool {
        let mut scales = self.monitors.iter().map(|m| m.scale_factor);
        match scales.next() {
            None => false,
            Some(first) => scales.any(|s| (s - first).abs() > 1e-6),
        }
    }

    /// 把一块屏的局部物理坐标转回 VSS。
    pub fn to_vss(monitor: &MonitorInfo, local_physical: Pos) -> Pos {
        Pos::new(
            local_physical.x + monitor.vss_rect.min.x,
            local_physical.y + monitor.vss_rect.min.y,
        )
    }
}

/// 单台显示器上的坐标转换器。
///
/// 每块遮罩窗口持有一个，用于在事件处理中把窗口逻辑坐标换回 VSS。
/// 封装成类型而不是裸函数，是为了避免"忘了减原点"这类错误 ——
/// 这正是多屏偏移 bug 的典型来源。
#[derive(Debug, Clone, PartialEq)]
pub struct VssToScreen {
    pub monitor: MonitorInfo,
}

impl VssToScreen {
    pub fn new(monitor: MonitorInfo) -> Self {
        Self { monitor }
    }

    /// 该屏在 VSS 中占据的矩形。
    pub fn region(&self) -> Rect {
        self.monitor.vss_rect
    }

    /// VSS 物理像素 → 本屏逻辑坐标（喂给 egui 画图）。
    pub fn vss_to_logical(&self, p: Pos) -> Pos {
        self.monitor.transform().apply(p)
    }

    /// 本屏逻辑坐标（来自鼠标事件）→ VSS 物理像素。
    pub fn logical_to_vss(&self, p: Pos) -> Pos {
        self.monitor.transform().invert(p)
    }

    /// VSS 矩形 → 本屏逻辑矩形。
    pub fn vss_rect_to_logical(&self, r: &Rect) -> Rect {
        Rect::from_points(self.vss_to_logical(r.min), self.vss_to_logical(r.max))
    }

    /// 本屏逻辑矩形 → VSS 矩形。
    pub fn logical_rect_to_vss(&self, r: &Rect) -> Rect {
        Rect::from_points(self.logical_to_vss(r.min), self.logical_to_vss(r.max))
    }

    /// VSS 矩形 → 本屏局部物理像素矩形（用于从帧缓冲里裁剪）。
    pub fn vss_rect_to_local_physical(&self, r: &Rect) -> Rect {
        Rect::from_points(
            self.monitor.to_local_physical(r.min),
            self.monitor.to_local_physical(r.max),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个典型的混合 DPI 双屏环境：
    /// 左屏 1920×1080@100%，右屏 3840×2160@200%，右屏紧贴左屏右侧。
    fn mixed_dpi_layout() -> VssLayout {
        VssLayout::new(vec![
            MonitorInfo {
                id: 1,
                name: "Left".into(),
                vss_rect: Rect::new(0.0, 0.0, 1920.0, 1080.0),
                scale_factor: 1.0,
                is_primary: true,
            },
            MonitorInfo {
                id: 2,
                name: "Right".into(),
                vss_rect: Rect::new(1920.0, 0.0, 3840.0, 2160.0),
                scale_factor: 2.0,
                is_primary: false,
            },
        ])
    }

    #[test]
    fn bounds_is_union_of_all_monitors() {
        let l = mixed_dpi_layout();
        assert_eq!(l.bounds.min, Pos::new(0.0, 0.0));
        assert_eq!(l.bounds.max, Pos::new(5760.0, 2160.0));
        assert_eq!(l.len(), 2);
        assert!(!l.is_empty());
    }

    #[test]
    fn monitor_hit_test() {
        let l = mixed_dpi_layout();
        assert_eq!(l.monitor_at(Pos::new(100.0, 100.0)).unwrap().id, 1);
        assert_eq!(l.monitor_at(Pos::new(3000.0, 1000.0)).unwrap().id, 2);
        // 恰好落在分界线上 → 归属左屏（先匹配到）
        assert_eq!(l.monitor_at(Pos::new(1920.0, 500.0)).unwrap().id, 1);
        // 超出所有屏
        assert!(l.monitor_at(Pos::new(9999.0, 9999.0)).is_none());
    }

    #[test]
    fn primary_falls_back_to_first() {
        let mut l = mixed_dpi_layout();
        assert_eq!(l.primary().unwrap().id, 1);
        // 没有标记主屏时，退化为第一块屏而不是 None
        for m in &mut l.monitors {
            m.is_primary = false;
        }
        assert_eq!(l.primary().unwrap().id, 1);
    }

    #[test]
    fn logical_size_accounts_for_scale() {
        let l = mixed_dpi_layout();
        // 200% 的 4K 屏，逻辑尺寸是 1920×1080
        assert_eq!(l.monitors[1].logical_size(), (1920.0, 1080.0));
        assert_eq!(l.monitors[0].logical_size(), (1920.0, 1080.0));
        // 两者物理尺寸不同但逻辑尺寸相同 —— 正是混合 DPI 的核心特征
        assert_eq!(l.monitors[1].physical_size(), (3840, 2160));
    }

    #[test]
    fn mixed_dpi_detection() {
        assert!(mixed_dpi_layout().has_mixed_dpi());
        let uniform = VssLayout::new(vec![MonitorInfo {
            id: 1,
            name: "Only".into(),
            vss_rect: Rect::new(0.0, 0.0, 2560.0, 1440.0),
            scale_factor: 1.0,
            is_primary: true,
        }]);
        assert!(!uniform.has_mixed_dpi());
    }

    #[test]
    fn transform_roundtrip_on_second_monitor() {
        let l = mixed_dpi_layout();
        let conv = VssToScreen::new(l.monitors[1].clone());

        // 右屏左上角在 VSS 中是 (1920, 0)，对应本屏逻辑坐标 (0, 0)
        assert_eq!(
            conv.vss_to_logical(Pos::new(1920.0, 0.0)),
            Pos::new(0.0, 0.0)
        );

        // 物理 (1920+200, 300) → 逻辑 (100, 150)
        let vss_pt = Pos::new(2120.0, 300.0);
        assert_eq!(conv.vss_to_logical(vss_pt), Pos::new(100.0, 150.0));

        // 往返一致 —— 这是多屏坐标不偏移的根本保证
        let back = conv.logical_to_vss(conv.vss_to_logical(vss_pt));
        assert!((back.x - vss_pt.x).abs() < 1e-3 && (back.y - vss_pt.y).abs() < 1e-3);
    }

    #[test]
    fn first_monitor_transform_is_identity_at_1x() {
        let l = mixed_dpi_layout();
        let conv = VssToScreen::new(l.monitors[0].clone());
        // 100% 缩放、原点在 VSS (0,0) → 恒等变换
        let p = Pos::new(123.0, 456.0);
        assert_eq!(conv.vss_to_logical(p), p);
    }

    #[test]
    fn cross_monitor_selection_reports_both() {
        let l = mixed_dpi_layout();
        // 横跨分界线的选区
        let sel = Rect::new(1800.0, 0.0, 400.0, 600.0);
        let covered = l.monitors_covering(&sel);
        assert_eq!(covered.len(), 2);
        assert_eq!(covered[0].id, 1);
        assert_eq!(covered[1].id, 2);
    }

    #[test]
    fn single_monitor_selection_reports_one() {
        let l = mixed_dpi_layout();
        let sel = Rect::new(100.0, 100.0, 200.0, 200.0);
        let covered = l.monitors_covering(&sel);
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].id, 1);
    }

    #[test]
    fn local_physical_to_vss_roundtrip() {
        let l = mixed_dpi_layout();
        let m = &l.monitors[1];
        let local = Pos::new(500.0, 400.0);
        let vss = VssLayout::to_vss(m, local);
        assert_eq!(vss, Pos::new(2420.0, 400.0));
        assert_eq!(m.to_local_physical(vss), local);
    }

    #[test]
    fn empty_layout_is_safe() {
        let l = VssLayout::new(vec![]);
        assert!(l.is_empty());
        assert!(l.primary().is_none());
        assert!(l.monitor_at(Pos::ZERO).is_none());
        assert!(!l.has_mixed_dpi());
    }
}
