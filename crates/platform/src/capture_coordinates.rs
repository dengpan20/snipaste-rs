//! Single-display Quartz point to physical-pixel mapping, independent of AppKit.
use snipaste_core::{Pos, Rect};

#[derive(Clone, Copy)]
pub struct ScreenSpace {
    pub logical: Rect,
    pub scale: f32,
}
impl ScreenSpace {
    pub fn to_physical(self, point: Pos) -> Pos {
        Pos::new(point.x * self.scale, point.y * self.scale)
    }
    pub fn to_logical(self, point: Pos) -> Pos {
        Pos::new(point.x / self.scale, point.y / self.scale)
    }
    pub fn physical_bounds(self) -> Rect {
        Rect::from_points(
            self.to_physical(self.logical.min),
            self.to_physical(self.logical.max),
        )
    }
    pub fn clip_window(self, logical: Rect) -> Option<Rect> {
        logical
            .intersect(&self.logical)
            .map(|rect| Rect::from_points(self.to_physical(rect.min), self.to_physical(rect.max)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negative_retina_origin_preserves_pixel_offsets() {
        let screen = ScreenSpace {
            logical: Rect::new(-1512.0, -982.0, 1512.0, 982.0),
            scale: 2.0,
        };
        let cursor = Pos::new(-1400.5, -900.0);
        let pixel = screen.to_physical(cursor);
        assert_eq!(screen.to_logical(pixel), cursor);
        assert_eq!(pixel - screen.physical_bounds().min, Pos::new(223.0, 164.0));
        assert_eq!(screen.physical_bounds().width(), 3024.0);
    }
    #[test]
    fn mixed_scale_screens_clip_windows_before_scaling() {
        let external = ScreenSpace {
            logical: Rect::new(-1920.0, 0.0, 1920.0, 1080.0),
            scale: 1.0,
        };
        let retina = ScreenSpace {
            logical: Rect::new(0.0, 0.0, 1512.0, 982.0),
            scale: 2.0,
        };
        let window = Rect::new(-200.0, 100.0, 500.0, 400.0);
        assert_eq!(
            external.clip_window(window),
            Some(Rect::new(-200.0, 100.0, 200.0, 400.0))
        );
        assert_eq!(
            retina.clip_window(window),
            Some(Rect::new(0.0, 200.0, 600.0, 800.0))
        );
        assert!(retina
            .clip_window(Rect::new(-800.0, 100.0, 100.0, 100.0))
            .is_none());
    }
}
