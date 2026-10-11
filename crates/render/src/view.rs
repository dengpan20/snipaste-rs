//! VSS ↔ egui 坐标换算。
//!
//! ## 为什么必须有这一层
//!
//! `snipaste-core` 里的一切坐标都是 **VSS 物理像素**（虚拟桌面左上角为原点）。
//! 而 egui 的 `Painter` 只认 **该窗口内的逻辑点**（左上角为原点、按
//! `pixels_per_point` 缩放）。
//!
//! 两者之间的换算如果散落在各处（画矩形时换一次、画箭头时又换一次），
//! 迟早会有一处漏掉 `scale` 或者漏掉 `origin`，在有缩放的多屏环境下表现为
//! "标注整体偏移了半个屏幕"。所以统一收进这一个类型。

use snipaste_core::geometry::{Pos, Rect};

/// 一块显示器上「遮罩窗口」的视图变换。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// 本窗口覆盖的显示器左上角在 VSS 中的位置（物理像素）。
    pub origin: Pos,
    /// 本屏的 pixels_per_point（物理像素 / 逻辑点）。
    pub scale: f32,
}

impl Viewport {
    /// `scale` 会被夹到正数 —— 传 0 进来只会让后续计算全变成 NaN 或 inf。
    pub fn new(origin: Pos, scale: f32) -> Self {
        Self {
            origin,
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
        }
    }

    /// VSS 物理像素 → 窗口内逻辑点。
    #[inline]
    pub fn to_points(&self, p: Pos) -> egui::Pos2 {
        egui::pos2(
            (p.x - self.origin.x) / self.scale,
            (p.y - self.origin.y) / self.scale,
        )
    }

    /// 窗口内逻辑点 → VSS 物理像素。
    #[inline]
    pub fn to_vss(&self, p: egui::Pos2) -> Pos {
        Pos {
            x: p.x * self.scale + self.origin.x,
            y: p.y * self.scale + self.origin.y,
        }
    }

    /// VSS 矩形 → 窗口内逻辑矩形。
    #[inline]
    pub fn rect_to_points(&self, r: &Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.to_points(r.min), self.to_points(r.max))
    }

    /// 窗口内逻辑矩形 → VSS 矩形。
    #[inline]
    pub fn rect_to_vss(&self, r: egui::Rect) -> Rect {
        Rect::from_points(self.to_vss(r.min), self.to_vss(r.max))
    }

    /// 长度换算：物理像素 → 逻辑点。
    ///
    /// **所有像素尺寸（线宽、字号、马赛克块）都必须走这里**，
    /// 否则在 150% 缩放的屏幕上笔迹会显得比设定的粗 1.5 倍。
    #[inline]
    pub fn len_to_points(&self, v: f32) -> f32 {
        v / self.scale
    }

    /// 长度换算：逻辑点 → 物理像素。
    #[inline]
    pub fn len_to_vss(&self, v: f32) -> f32 {
        v * self.scale
    }

    /// 一整屏的 VSS 外接矩形（即本窗口覆盖的范围）。
    pub fn screen_rect(&self, size_points: egui::Vec2) -> Rect {
        Rect::from_points(
            self.origin,
            self.to_vss(egui::pos2(size_points.x, size_points.y)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_at_100_percent() {
        let v = Viewport::new(Pos::new(1920.0, 0.0), 1.0);
        let p = Pos::new(2000.0, 300.0);
        let back = v.to_vss(v.to_points(p));
        assert!((back.x - p.x).abs() < 1e-3);
        assert!((back.y - p.y).abs() < 1e-3);
    }

    #[test]
    fn round_trip_at_150_percent_second_screen() {
        // 副屏 150% 缩放、位于主屏右侧
        let v = Viewport::new(Pos::new(1920.0, 0.0), 1.5);
        let p = Pos::new(2400.0, 600.0);
        let pt = v.to_points(p);
        assert!((pt.x - 320.0).abs() < 1e-3);
        assert!((pt.y - 400.0).abs() < 1e-3);
        let back = v.to_vss(pt);
        assert!((back.x - p.x).abs() < 1e-3);
        assert!((back.y - p.y).abs() < 1e-3);
    }

    #[test]
    fn length_scales_inversely() {
        let v = Viewport::new(Pos::ZERO, 2.0);
        // 4 物理像素的线宽在 200% 屏上是 2 个逻辑点
        assert!((v.len_to_points(4.0) - 2.0).abs() < 1e-6);
        assert!((v.len_to_vss(2.0) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn invalid_scale_falls_back_to_one() {
        assert_eq!(Viewport::new(Pos::ZERO, 0.0).scale, 1.0);
        assert_eq!(Viewport::new(Pos::ZERO, f32::NAN).scale, 1.0);
        assert_eq!(Viewport::new(Pos::ZERO, -3.0).scale, 1.0);
    }
}
