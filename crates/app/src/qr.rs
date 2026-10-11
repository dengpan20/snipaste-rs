//! 识别选区的原始像素，处理多个二维码与反色码；不自动访问识别出的链接。
pub fn recognize(image: &image::RgbaImage) -> Result<Vec<String>, String> {
    let gray = image::imageops::grayscale(image);
    let mut result = Vec::new();
    for inverted in [false, true] {
        let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(
            gray.width() as usize,
            gray.height() as usize,
            |x, y| {
                let value = gray.get_pixel(x as u32, y as u32)[0];
                if inverted {
                    255 - value
                } else {
                    value
                }
            },
        );
        for grid in prepared.detect_grids() {
            if let Ok((_, content)) = grid.decode() {
                if !result.contains(&content) {
                    result.push(content);
                }
            }
        }
    }
    if result.is_empty() {
        Err("选区内未识别到二维码，请完整框选二维码及周围的空白区域后重试。".into())
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snipaste_capture::Snapshot;
    use snipaste_core::{Pos, Rect};
    fn fixture(content: &str) -> image::RgbaImage {
        let qr = qrcode::QrCode::new(content).unwrap();
        let modules = qr.width() as u32;
        image::RgbaImage::from_fn((modules + 8) * 6, (modules + 8) * 6, |x, y| {
            let (x, y) = (x as i32 / 6 - 4, y as i32 / 6 - 4);
            let dark = x >= 0
                && y >= 0
                && x < modules as i32
                && y < modules as i32
                && qr[(x as usize, y as usize)] == qrcode::Color::Dark;
            image::Rgba(if dark { [0, 0, 0, 255] } else { [255; 4] })
        })
    }
    #[test]
    fn only_decodes_codes_inside_selected_region() {
        let first = fixture("https://example.com/selected");
        let second = fixture("outside-selection");
        let mut desktop = image::RgbaImage::from_pixel(700, 300, image::Rgba([255; 4]));
        image::imageops::overlay(&mut desktop, &first, 10, 10);
        image::imageops::overlay(&mut desktop, &second, 400, 10);
        let shot = Snapshot {
            origin: Pos::new(-700.0, 0.0),
            image: desktop,
        };
        let crop = shot.crop(Rect::new(-700.0, 0.0, 320.0, 300.0)).unwrap();
        assert_eq!(
            recognize(&crop.image).unwrap(),
            vec!["https://example.com/selected"]
        );
        assert_eq!(recognize(&shot.image).unwrap().len(), 2);
    }
    #[test]
    fn inverted_qr_and_no_qr_have_clear_results() {
        let mut image = fixture("二维码测试");
        for pixel in image.pixels_mut() {
            for c in 0..3 {
                pixel[c] = 255 - pixel[c];
            }
        }
        assert_eq!(recognize(&image).unwrap(), vec!["二维码测试"]);
        assert!(recognize(&image::RgbaImage::from_pixel(
            120,
            120,
            image::Rgba([255; 4])
        ))
        .is_err());
    }

    #[test]
    #[ignore = "手动生成原生验收所用二维码图片"]
    fn write_acceptance_fixture() {
        let path = std::env::var("SNIPASTE_QR_FIXTURE_PATH").expect("请提供测试图片路径");
        fixture("https://example.com/snipaste-test")
            .save(path)
            .unwrap();
    }
}
