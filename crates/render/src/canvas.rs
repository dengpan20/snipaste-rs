//! 把 [`Document`] 画到 egui 的 [`Painter`] 上。
//!
//! ## 一条硬规则：几何必须与导出路径共用
//!
//! 箭头折线走 [`arrow_polyline`]、椭圆轮廓走 [`ellipse_outline`]、笔迹抽稀走
//! `snipaste_core::simplify`。**渲染层不允许自己再实现一套几何**，否则会出现
//! "预览看着对齐、导出却错位"这类极难排查的问题。
//!
//! 渲染层只做两件事：坐标换算（交给 [`Viewport`]）和把矢量点喂给 epaint。

use egui::{Align2, Color32, FontId, Painter, Pos2, Shape, Stroke, StrokeKind};
use snipaste_core::annotation::{arrow_polyline, ellipse_outline, MosaicRegion};
use snipaste_core::geometry::{Pos, Rect};
use snipaste_core::style::{ArrowHead, Color, MosaicMode, Style};
use snipaste_core::{Annotation, Document, PathKind};

use crate::theme;
use crate::view::Viewport;

/// 椭圆轮廓采样段数。72 段在 4K 屏上也看不出折线感。
pub const ELLIPSE_SEGMENTS: usize = 72;

/// 选区边框宽度（逻辑点）。
const SELECTION_BORDER: f32 = 1.5;
// 角柄尺寸不在这里定义 —— 用 `snipaste_core::HANDLE_SIZE`。
// 它必须与命中测试（`Rect::handle_at` + `HANDLE_HIT_RADIUS`）共用同一个值，
// 否则会出现"看得到角柄却抓不住"这种极难排查的手感问题。

/// `snipaste_core::Color` → egui 颜色。
///
/// 两者都是**非预乘**语义，直接对应，不需要任何转换。
#[inline]
pub fn to_color32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// 描边样式：线宽是物理像素，必须按屏幕缩放换算成逻辑点。
#[inline]
fn stroke_of(style: &Style, view: &Viewport) -> Stroke {
    Stroke::new(
        view.len_to_points(style.stroke_width),
        to_color32(style.stroke),
    )
}

#[inline]
fn fill_of(style: &Style) -> Option<Color32> {
    style.fill.filter(|c| !c.is_transparent()).map(to_color32)
}

#[inline]
fn to_pts(points: &[Pos], view: &Viewport) -> Vec<Pos2> {
    points.iter().map(|p| view.to_points(*p)).collect()
}

// ------------------------------------------------------------------ 单个图元

/// 画一个图元。调用方负责已经设置好 clip rect。
pub fn paint_annotation(painter: &Painter, ann: &Annotation, view: &Viewport) {
    match ann {
        Annotation::Rect { rect, style } => paint_rect(painter, rect, style, view),
        Annotation::Ellipse { rect, style } => paint_ellipse(painter, rect, style, view),
        Annotation::Arrow {
            from,
            to,
            kind,
            style,
        } => paint_arrow(painter, *from, *to, *kind, style, view),
        Annotation::Path {
            points,
            kind,
            style,
        } => paint_freehand(painter, points, *kind, style, view),
        // 像素效果必须有底图，统一走 paint_document_with_image。
        Annotation::Mosaic { .. } | Annotation::Eraser { .. } => {}
        Annotation::Text {
            rect,
            content,
            style,
        } => paint_text(painter, rect, content, style, view),
    }
}

fn paint_rect(painter: &Painter, rect: &Rect, style: &Style, view: &Viewport) {
    let r = view.rect_to_points(rect);
    if let Some(fill) = fill_of(style) {
        painter.rect_filled(r, 0.0, fill);
    }
    painter.add(Shape::rect_stroke(
        r,
        0.0,
        stroke_of(style, view),
        StrokeKind::Middle,
    ));
}

fn paint_ellipse(painter: &Painter, rect: &Rect, style: &Style, view: &Viewport) {
    let r = view.rect_to_points(rect);
    let radius = r.size() * 0.5;
    if let Some(fill) = fill_of(style) {
        painter.add(Shape::ellipse_filled(r.center(), radius, fill));
    }
    // 用 core 的采样点而不是 Shape::ellipse_stroke：
    // 保证预览的轮廓与命中测试 / 导出走的是同一批点。
    let outline = ellipse_outline(rect, ELLIPSE_SEGMENTS);
    painter.add(Shape::closed_line(
        to_pts(&outline, view),
        stroke_of(style, view),
    ));
}

fn paint_arrow(
    painter: &Painter,
    from: Pos,
    to: Pos,
    kind: snipaste_core::ArrowKind,
    style: &Style,
    view: &Viewport,
) {
    let poly = arrow_polyline(from, to, kind);
    if poly.len() < 2 {
        return;
    }
    painter.add(Shape::line(to_pts(&poly, view), stroke_of(style, view)));

    // 箭头头的朝向要沿**最后一段**的切线，而不是 from→to 的连线。
    // 曲线箭头如果按连线算，箭头头会明显歪掉。
    let tail = poly[poly.len() - 2];
    if let Some(head) = ArrowHead::build(tail, to, style.stroke_width) {
        painter.add(Shape::convex_polygon(
            to_pts(&[head.tip, head.left, head.right], view),
            to_color32(style.stroke),
            Stroke::NONE,
        ));
    }
}

fn paint_freehand(
    painter: &Painter,
    points: &[Pos],
    kind: PathKind,
    style: &Style,
    view: &Viewport,
) {
    if points.len() < 2 {
        return;
    }
    let stroke = stroke_of(style, view);
    match kind {
        PathKind::Pen => {
            painter.add(Shape::line(to_pts(points, view), stroke));
        }
        PathKind::Marker => {
            // 记号笔：更粗、更透（模拟马克笔叠色的半透明感）。
            //
            // TODO(渲染增强)：epaint 的 Stroke 没有 line-cap / line-join 控制，
            // 现在画出来是圆头圆角；真·记号笔的方头斜接需要自定义 mesh。
            // 这不影响几何与撤销栈，只影响观感，因此放到渲染打磨阶段。
            let marker = Stroke::new(
                view.len_to_points(style.stroke_width * 2.0),
                to_color32(style.stroke.with_alpha_f(0.45)),
            );
            painter.add(Shape::line(to_pts(points, view), marker));
        }
    }
}

fn paint_text(painter: &Painter, rect: &Rect, content: &str, style: &Style, view: &Viewport) {
    if content.is_empty() {
        return;
    }
    let pos = view.to_points(rect.min);
    painter.text(
        pos,
        Align2::LEFT_TOP,
        content,
        FontId::proportional(view.len_to_points(style.font_size)),
        to_color32(style.stroke),
    );
}

// ------------------------------------------------------------------ 整篇文档

/// 画出所有已提交图元 + 当前草稿。**不画选区**（选区单独画，见下）。
pub fn paint_document(painter: &Painter, doc: &Document, view: &Viewport) {
    for item in &doc.items {
        paint_annotation(painter, item, view);
    }
    if let Some(draft) = &doc.draft {
        paint_annotation(painter, draft, view);
    }
}

/// 冻结底图及其 GPU 纹理。预览与导出都使用同一份原图、相同 VSS 网格。
#[derive(Clone, Copy)]
pub struct SourceImage<'a> {
    pub image: &'a image::RgbaImage,
    pub origin: Pos,
    pub texture: egui::TextureId,
}

#[derive(Clone, Default)]
struct MosaicTextures {
    source: Option<egui::TextureId>,
    entries: Vec<(MosaicMode, u32, egui::TextureHandle, [u32; 2])>,
}

pub fn clear_image_cache(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<MosaicTextures>(egui::Id::new("canvas.mosaic")));
}

fn effect_texture(
    painter: &Painter,
    source: SourceImage<'_>,
    style: &Style,
) -> (egui::TextureId, [u32; 2]) {
    let key = egui::Id::new("canvas.mosaic");
    let mut cache = painter
        .ctx()
        .data(|data| data.get_temp::<MosaicTextures>(key))
        .unwrap_or_default();
    if cache.source != Some(source.texture) {
        cache.entries.clear();
        cache.source = Some(source.texture);
    }
    let block = style.mosaic_block.clamp(4, 40);
    if let Some((_, _, texture, extent)) = cache
        .entries
        .iter()
        .find(|(mode, size, _, _)| *mode == style.mosaic_mode && *size == block)
    {
        return (texture.id(), *extent);
    }
    // 不持有 Context 的 data 锁时创建纹理，load_texture 自己需要读取 Context。
    let processed = match style.mosaic_mode {
        MosaicMode::Pixelate => pixelate(source.image, block),
        MosaicMode::Blur => image::imageops::blur(source.image, block as f32 * 0.5),
    };
    let extent = if style.mosaic_mode == MosaicMode::Pixelate {
        [processed.width() * block, processed.height() * block]
    } else {
        [processed.width(), processed.height()]
    };
    let texture = painter.ctx().load_texture(
        "canvas.mosaic.pixels",
        egui::ColorImage::from_rgba_unmultiplied(
            [processed.width() as usize, processed.height() as usize],
            processed.as_raw(),
        ),
        egui::TextureOptions::NEAREST,
    );
    let id = texture.id();
    if cache.entries.len() == 4 {
        let _ = cache.entries.remove(0);
    }
    cache
        .entries
        .push((style.mosaic_mode, block, texture, extent));
    painter.ctx().data_mut(|data| data.insert_temp(key, cache));
    (id, extent)
}

fn pixelate(image: &image::RgbaImage, block: u32) -> image::RgbaImage {
    // 每个马赛克块仅存一个平均像素，由最近邻采样铺开；避免保留整屏副本。
    let mut result = image::RgbaImage::new(
        image.width().div_ceil(block),
        image.height().div_ceil(block),
    );
    for y in (0..image.height()).step_by(block as usize) {
        for x in (0..image.width()).step_by(block as usize) {
            let right = (x + block).min(image.width());
            let bottom = (y + block).min(image.height());
            let mut sum = [0u32; 4];
            for sy in y..bottom {
                for sx in x..right {
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += image.get_pixel(sx, sy)[c] as u32;
                    }
                }
            }
            let count = (right - x) * (bottom - y);
            let average = image::Rgba(sum.map(|total| (total / count) as u8));
            result.put_pixel(x / block, y / block, average);
        }
    }
    result
}

pub fn paint_document_with_image(
    painter: &Painter,
    doc: &Document,
    view: &Viewport,
    source: SourceImage<'_>,
) {
    for ann in doc.items.iter().chain(doc.draft.iter()) {
        match ann {
            Annotation::Mosaic { region, style } => {
                let (texture, extent) = effect_texture(painter, source, style);
                paint_image_region(
                    painter,
                    region,
                    style.stroke_width,
                    view,
                    source,
                    texture,
                    extent,
                );
            }
            Annotation::Eraser { region, style } => {
                paint_image_region(
                    painter,
                    region,
                    style.stroke_width,
                    view,
                    source,
                    source.texture,
                    [source.image.width(), source.image.height()],
                );
            }
            _ => paint_annotation(painter, ann, view),
        }
    }
}

/// 笔刷由连续线段及圆形端点组成，快速移动与单击都能覆盖准确的局部区域。
/// UV 按原图坐标计算，擦除不会平移/拉伸底图，也不会抹掉整条标注。
fn paint_image_region(
    painter: &Painter,
    region: &MosaicRegion,
    width: f32,
    view: &Viewport,
    source: SourceImage<'_>,
    texture: egui::TextureId,
    texture_extent: [u32; 2],
) {
    let mut mesh = egui::Mesh::with_texture(texture);
    let mut polygon = |points: &[Pos]| {
        let base = mesh.vertices.len() as u32;
        for p in points {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: view.to_points(*p),
                uv: egui::pos2(
                    (p.x - source.origin.x) / texture_extent[0] as f32,
                    (p.y - source.origin.y) / texture_extent[1] as f32,
                ),
                color: Color32::WHITE,
            });
        }
        for i in 1..points.len().saturating_sub(1) {
            mesh.indices
                .extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
        }
    };
    match region {
        MosaicRegion::Rect(r) => {
            if !r.is_empty() {
                polygon(&[
                    r.min,
                    Pos::new(r.max.x, r.min.y),
                    r.max,
                    Pos::new(r.min.x, r.max.y),
                ]);
            }
        }
        MosaicRegion::Path(points) => {
            let radius = width.max(1.0) * 0.5;
            for pair in points.windows(2) {
                let delta = pair[1] - pair[0];
                let length = delta.length();
                if length > f32::EPSILON {
                    let normal = Pos::new(-delta.y * radius / length, delta.x * radius / length);
                    polygon(&[
                        pair[0] + normal,
                        pair[1] + normal,
                        pair[1] - normal,
                        pair[0] - normal,
                    ]);
                }
            }
            for p in points {
                let circle: Vec<Pos> = (0..32)
                    .map(|i| {
                        let angle = i as f32 * std::f32::consts::TAU / 32.0;
                        *p + Pos::new(angle.cos() * radius, angle.sin() * radius)
                    })
                    .collect();
                polygon(&circle);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// 把选区之外的部分压暗。
///
/// 用"四块矩形"而不是一次整屏 fill + 挖洞：egui 没有做布尔减法的便捷 API，
/// 而四块矩形在任何情况下都不会有半透明叠加的接缝问题（它们互不重叠）。
pub fn paint_dim_outside(painter: &Painter, screen: egui::Rect, hole: Option<egui::Rect>) {
    let Some(hole) = hole else {
        painter.rect_filled(screen, 0.0, theme::DIM_OVERLAY);
        return;
    };
    let h = hole.intersect(screen);
    let bands = [
        egui::Rect::from_min_max(screen.min, egui::pos2(screen.max.x, h.min.y)),
        egui::Rect::from_min_max(egui::pos2(screen.min.x, h.max.y), screen.max),
        egui::Rect::from_min_max(
            egui::pos2(screen.min.x, h.min.y),
            egui::pos2(h.min.x, h.max.y),
        ),
        egui::Rect::from_min_max(
            egui::pos2(h.max.x, h.min.y),
            egui::pos2(screen.max.x, h.max.y),
        ),
    ];
    for band in bands {
        if band.is_positive() {
            painter.rect_filled(band, 0.0, theme::DIM_OVERLAY);
        }
    }
}

/// 画选区的边框、八个角柄与尺寸标签。
pub fn paint_selection_frame(painter: &Painter, sel: &Rect, view: &Viewport) {
    let r = view.rect_to_points(sel);
    if !r.is_positive() {
        return;
    }

    painter.add(Shape::rect_stroke(
        r,
        0.0,
        Stroke::new(SELECTION_BORDER, theme::ACCENT),
        StrokeKind::Inside,
    ));

    // 角柄：用户能看出"这个选区是可以拖的"。
    // 位置走 core 的 `Handle::position_on`、尺寸走 core 的 `HANDLE_SIZE` ——
    // 与命中测试 `Rect::handle_at(p, HANDLE_HIT_RADIUS)` 是同一套常量，
    // 从根上排除"画出来的位置/大小和点得中的对不上"。
    let handle_rect = |p: egui::Pos2| {
        egui::Rect::from_center_size(
            p,
            egui::vec2(snipaste_core::HANDLE_SIZE, snipaste_core::HANDLE_SIZE),
        )
    };
    for handle in snipaste_core::Handle::ALL {
        let p = view.to_points(handle.position_on(sel));
        painter.rect_filled(handle_rect(p), 0.0, Color32::WHITE);
        painter.add(Shape::rect_stroke(
            handle_rect(p),
            0.0,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Inside,
        ));
    }

    // 尺寸标签。物理像素才是用户真正关心的数字（导出尺寸就按它算）。
    let label = format!(
        "{} × {}",
        sel.width().round() as i64,
        sel.height().round() as i64
    );
    let galley = painter.layout_no_wrap(label, FontId::proportional(12.0), Color32::WHITE);
    let size = galley.size();
    // 贴在选区左上角外侧。`anchor_size` 返回的是"按锚点摆好的矩形"，
    // 取它的 min 就得到文字左上角的实际位置。
    let anchor = Align2::LEFT_BOTTOM;
    let mut placed = anchor.anchor_size(r.left_top() + egui::vec2(0.0, -4.0), size);
    if placed.top() < painter.clip_rect().top() + 3.0 {
        placed = placed.translate(egui::vec2(0.0, r.top() + 8.0 - placed.top()));
    }
    if placed.left() < painter.clip_rect().left() + 5.0 {
        placed = placed.translate(egui::vec2(
            painter.clip_rect().left() + 5.0 - placed.left(),
            0.0,
        ));
    }
    let bg = placed.expand2(egui::vec2(5.0, 3.0));
    painter.rect_filled(bg, 2.0, Color32::from_rgba_unmultiplied(20, 22, 26, 220));
    painter.galley(placed.min, galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_conversion_is_faithful() {
        // core 的 `Color` 与 egui 的 `Color32` 都是**非预乘**语义，
        // 但 `Color32` 内部按 8 位预乘存储，所以半透明色会有一点点量化误差。
        // 这里用 ±1 的容差表达"语义一致但有量化损失"，而不是断言完全无损 ——
        // 后者是一个错误的期望，会让人以为可以做逐位比较。
        let c = Color::rgba(12, 34, 56, 78);
        let got = to_color32(c).to_srgba_unmultiplied();
        for (i, (g, want)) in got.iter().zip([12u8, 34, 56, 78]).enumerate() {
            let diff = (*g as i32 - want as i32).abs();
            assert!(diff <= 1, "通道 {i}：得到 {g}，期望 {want}");
        }

        // 全不透明时预乘是恒等变换，逐位精确 —— 截图标注绝大多数都是不透明色
        let opaque = Color::rgb(232, 66, 66);
        let e = to_color32(opaque);
        assert_eq!((e.r(), e.g(), e.b(), e.a()), (232, 66, 66, 255));
    }

    #[test]
    fn pixelation_averages_real_detail_including_partial_edge_blocks() {
        let source = image::RgbaImage::from_fn(7, 5, |x, y| {
            image::Rgba([if (x + y) % 2 == 0 { 0 } else { 255 }, 20, 200, 255])
        });
        let result = pixelate(&source, 4);
        assert_eq!(result.dimensions(), (2, 2));
        assert_eq!(result.get_pixel(0, 0).0, [127, 20, 200, 255]);
        assert_eq!(result.get_pixel(1, 1).0, [85, 20, 200, 255]);
        assert_ne!(result, source);
    }
}
