//! 截图会话持有冻结的桌面像素。显示遮罩之前捕获，之后的裁剪与识别均读这份底图。
use image::RgbaImage;
use snipaste_core::{Pos, Rect};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("截图区域为空或超出底图")]
    InvalidRegion,
    #[error("屏幕捕获失败：{0}")]
    Platform(String),
    #[error("请在系统设置中允许映贴录制屏幕，然后退出并重新打开应用")]
    PermissionDenied,
    #[error("当前平台尚未提供屏幕捕获")]
    Unsupported,
}

pub struct Snapshot {
    pub origin: Pos,
    pub image: RgbaImage,
}

impl Snapshot {
    pub fn bounds(&self) -> Rect {
        Rect::new(
            self.origin.x,
            self.origin.y,
            self.image.width() as f32,
            self.image.height() as f32,
        )
    }

    /// 向外取整保证选区中的每个物理像素都被保留，负原点与 DPI 不改变裁剪规则。
    pub fn crop(&self, selection: Rect) -> Result<Self, CaptureError> {
        let r = self.crop_bounds(selection)?;
        let x = (r.min.x - self.origin.x) as u32;
        let y = (r.min.y - self.origin.y) as u32;
        Ok(Self {
            origin: r.min,
            image: image::imageops::crop_imm(
                &self.image,
                x,
                y,
                r.width() as u32,
                r.height() as u32,
            )
            .to_image(),
        })
    }

    /// 只计算物理像素范围；合成标注时无需先复制一份待覆盖的底图。
    pub fn crop_bounds(&self, selection: Rect) -> Result<Rect, CaptureError> {
        let r = selection
            .intersect(&self.bounds())
            .filter(|r| !r.is_empty())
            .ok_or(CaptureError::InvalidRegion)?;
        let x = (r.min.x - self.origin.x).floor().max(0.0) as u32;
        let y = (r.min.y - self.origin.y).floor().max(0.0) as u32;
        let right = ((r.max.x - self.origin.x).ceil() as u32).min(self.image.width());
        let bottom = ((r.max.y - self.origin.y).ceil() as u32).min(self.image.height());
        if right <= x || bottom <= y {
            return Err(CaptureError::InvalidRegion);
        }
        Ok(Rect::new(
            self.origin.x + x as f32,
            self.origin.y + y as f32,
            (right - x) as f32,
            (bottom - y) as f32,
        ))
    }
}

pub fn capture(bounds: Rect) -> Result<Snapshot, CaptureError> {
    imp::capture(bounds)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::*;

    pub fn capture(bounds: Rect) -> Result<Snapshot, CaptureError> {
        let w = bounds.width().round() as i32;
        let h = bounds.height().round() as i32;
        if w <= 0 || h <= 0 {
            return Err(CaptureError::InvalidRegion);
        }
        unsafe {
            let desktop = GetDC(null_mut());
            if desktop.is_null() {
                return Err(failed());
            }
            let dc = CreateCompatibleDC(desktop);
            if dc.is_null() {
                ReleaseDC(null_mut(), desktop);
                return Err(failed());
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(desktop, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                ReleaseDC(null_mut(), desktop);
                return Err(failed());
            }
            let old = SelectObject(dc, bitmap);
            let ok = BitBlt(
                dc,
                0,
                0,
                w,
                h,
                desktop,
                bounds.min.x as i32,
                bounds.min.y as i32,
                SRCCOPY | CAPTUREBLT,
            );
            GdiFlush();
            let mut rgba = Vec::new();
            if ok != 0 && !bits.is_null() {
                rgba = std::slice::from_raw_parts(bits.cast::<u8>(), w as usize * h as usize * 4)
                    .to_vec();
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
            }
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(null_mut(), desktop);
            if rgba.is_empty() {
                return Err(failed());
            }
            let image =
                RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or(CaptureError::InvalidRegion)?;
            Ok(Snapshot {
                origin: bounds.min,
                image,
            })
        }
    }
    fn failed() -> CaptureError {
        CaptureError::Platform(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use snipaste_platform::macos::{self, CaptureFailure};
    pub fn capture(bounds: Rect) -> Result<Snapshot, CaptureError> {
        let captured = macos::capture(bounds).map_err(|error| match error {
            CaptureFailure::PermissionDenied => CaptureError::PermissionDenied,
            CaptureFailure::Other(message) => CaptureError::Platform(message),
        })?;
        let image = RgbaImage::from_raw(captured.width, captured.height, captured.bytes)
            .ok_or(CaptureError::InvalidRegion)?;
        Ok(Snapshot {
            origin: bounds.min,
            image,
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;
    pub fn capture(_: Rect) -> Result<Snapshot, CaptureError> {
        Err(CaptureError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_uses_physical_pixels_and_negative_origin() {
        let shot = Snapshot {
            origin: Pos::new(-100.0, -50.0),
            image: RgbaImage::from_fn(20, 20, |x, y| image::Rgba([x as u8, y as u8, 42, 255])),
        };
        let crop = shot.crop(Rect::new(-97.5, -46.5, 6.0, 8.0)).unwrap();
        assert_eq!(crop.origin, Pos::new(-98.0, -47.0));
        assert_eq!(crop.image.dimensions(), (7, 9));
        assert_eq!(crop.image.get_pixel(0, 0).0, [2, 3, 42, 255]);
        assert!(shot.crop(Rect::new(10.0, 10.0, 2.0, 2.0)).is_err());
    }
}
