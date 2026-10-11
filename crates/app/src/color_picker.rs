//! 放大镜显示与复制共用原图像素和颜色格式，屏幕坐标均为物理像素。
use snipaste_capture::Snapshot;
use snipaste_core::Pos;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ColorFormat {
    #[default]
    Hex,
    Rgb,
}

impl ColorFormat {
    pub fn toggle(&mut self) {
        *self = match self {
            Self::Hex => Self::Rgb,
            Self::Rgb => Self::Hex,
        };
    }

    pub fn value(self, [r, g, b]: [u8; 3]) -> String {
        match self {
            Self::Hex => format!("#{r:02X}{g:02X}{b:02X}"),
            Self::Rgb => format!("{r}, {g}, {b}"),
        }
    }

    pub fn label(self, rgb: [u8; 3]) -> String {
        match self {
            Self::Hex => format!("HEX: {}", self.value(rgb)),
            Self::Rgb => format!("RGB: ({})", self.value(rgb)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickedColor {
    pub position: Pos,
    pub pixel: [u32; 2],
    pub rgb: [u8; 3],
}

pub fn sample(snapshot: &Snapshot, position: Pos) -> Option<PickedColor> {
    let local = position - snapshot.origin;
    if !local.x.is_finite()
        || !local.y.is_finite()
        || local.x < 0.0
        || local.y < 0.0
        || local.x >= snapshot.image.width() as f32
        || local.y >= snapshot.image.height() as f32
    {
        return None;
    }
    let x = local.x.floor() as u32;
    let y = local.y.floor() as u32;
    let color = snapshot.image.get_pixel(x, y);
    Some(PickedColor {
        position: snapshot.origin + Pos::new(x as f32, y as f32),
        pixel: [x, y],
        rgb: [color[0], color[1], color[2]],
    })
}

/// 17×17 像素的采样范围；中心对准当前像素中心，避免中心线落在两像素之间。
pub fn zoom_uv(snapshot: &Snapshot, pixel: [u32; 2]) -> egui::Rect {
    let [x, y] = pixel.map(|p| p as f32);
    let w = snapshot.image.width() as f32;
    let h = snapshot.image.height() as f32;
    egui::Rect::from_min_max(
        egui::pos2((x - 8.0) / w, (y - 8.0) / h),
        egui::pos2((x + 9.0) / w, (y + 9.0) / h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use snipaste_render::Viewport;

    #[test]
    fn values_preserve_channels_and_hex_leading_zeroes() {
        assert_eq!(ColorFormat::Hex.value([0, 5, 255]), "#0005FF");
        assert_eq!(ColorFormat::Rgb.value([0, 5, 255]), "0, 5, 255");
        assert_eq!(ColorFormat::Rgb.label([18, 52, 86]), "RGB: (18, 52, 86)");
    }

    #[test]
    fn sampling_and_zoom_agree_at_hidpi_and_negative_screen_origin() {
        let origin = Pos::new(-120.0, -40.0);
        let snapshot = Snapshot {
            origin,
            image: image::RgbaImage::from_fn(20, 20, |x, y| {
                image::Rgba([x as u8, y as u8, 86, 255])
            }),
        };
        let view = Viewport::new(origin, 2.0);
        let picked = sample(&snapshot, view.to_vss(egui::pos2(4.6, 3.7))).unwrap();
        assert_eq!(picked.pixel, [9, 7]);
        assert_eq!(picked.position, Pos::new(-111.0, -33.0));
        assert_eq!(picked.rgb, [9, 7, 86]);
        let uv = zoom_uv(&snapshot, picked.pixel);
        assert!((uv.center().x * 20.0 - 9.5).abs() < 0.0001);
        assert!((uv.center().y * 20.0 - 7.5).abs() < 0.0001);
        for p in [
            Pos::new(-121.0, -40.0),
            Pos::new(-100.0, -40.0),
            Pos::new(f32::NAN, 0.0),
        ] {
            assert!(
                sample(&snapshot, p).is_none(),
                "区域外指针不能错误地吸到边缘颜色"
            );
        }
    }
}
