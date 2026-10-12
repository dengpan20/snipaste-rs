//! 手动原生验收：截图底图、GPU 合成、界面截图与系统剪贴板，常规启动不执行。
use crate::{editor::Editor, export, gfx::GpuContext, ui};
use anyhow::{Context as _, Result};
use snipaste_capture::Snapshot;
use snipaste_core::{Annotation, Pos, Rect, Style, Tool};
use snipaste_render::{fonts, theme, Viewport};

fn same_pixel(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> bool {
    // GPU 插值与 UNORM 量化可能产生 1 级误差，不允许错色或错位。
    a.0.iter().zip(b.0).all(|(a, b)| a.abs_diff(b) <= 1)
}

pub fn fixture() -> Result<Snapshot> {
    let mut image = image::RgbaImage::from_fn(1100, 760, |x, y| {
        image::Rgba([
            205 + (x / 80 % 3) as u8 * 8,
            218 + (y / 90 % 3) as u8 * 6,
            235,
            255,
        ])
    });
    let qr = image::open(std::env::var("SNIPASTE_QR_FIXTURE_PATH")?)?.to_rgba8();
    image::imageops::overlay(&mut image, &qr, 600, 190);
    // 高频细节揭露“有马赛克图元却没有改变像素”的缺陷。
    for y in 425..510 {
        for x in 200..425 {
            image.put_pixel(
                x,
                y,
                image::Rgba(if (x + y) % 2 == 0 {
                    [20, 40, 180, 255]
                } else {
                    [240, 220, 40, 255]
                }),
            );
        }
    }
    Ok(Snapshot {
        origin: Pos::ZERO,
        image,
    })
}

pub fn editor() -> Editor {
    let mut ed = Editor::new();
    ed.reset_session(Rect::new(0.0, 0.0, 1100.0, 760.0), false);
    ed.set_selection(Some(Rect::new(160.0, 120.0, 760.0, 450.0)));
    ed.doc.items.push(Annotation::Rect {
        rect: Rect::new(215.0, 180.0, 200.0, 130.0),
        style: Style::default(),
    });
    ed.doc.items.push(Annotation::Arrow {
        from: Pos::new(480.0, 320.0),
        to: Pos::new(575.0, 250.0),
        kind: snipaste_core::ArrowKind::Straight,
        style: Style::default(),
    });
    ed.doc.items.push(Annotation::Text {
        rect: Rect::new(215.0, 350.0, 350.0, 60.0),
        content: ui::shortcut_hint("截图标注 · 确认后 Ctrl+V 粘贴", "截图标注 · 确认后 ⌘V 粘贴")
            .into(),
        style: Style {
            font_size: 26.0,
            ..Style::default()
        },
    });
    ed.set_tool(Tool::Mosaic);
    ed.begin(Pos::new(215.0, 440.0));
    ed.extend(Pos::new(330.0, 475.0));
    ed.finish(Pos::new(405.0, 490.0));
    ed
}

pub fn render_ui(
    gfx: &GpuContext,
    shot: &Snapshot,
    ed: &mut Editor,
    path: &std::path::Path,
) -> Result<()> {
    render_ui_with_input(gfx, shot, ed, path, egui::RawInput::default())
}

fn render_ui_with_input(
    gfx: &GpuContext,
    shot: &Snapshot,
    ed: &mut Editor,
    path: &std::path::Path,
    mut input: egui::RawInput,
) -> Result<()> {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    fonts::install(&ctx);
    ctx.set_pixels_per_point(1.0);
    let texture = ctx.load_texture(
        "acceptance.background",
        export::color_image(&shot.image),
        egui::TextureOptions::NEAREST,
    );
    input.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1100.0, 760.0),
    ));
    let mut full = ctx.run_ui(input.clone(), |ui| {
        ui::draw(
            ui,
            ed,
            Viewport::new(Pos::ZERO, 1.0),
            Some(ui::Background {
                snapshot: shot,
                texture: &texture,
            }),
            ui::shortcut_hint("F1", "⌘⇧2"),
        );
    });
    // Area 的首帧测量完成后，保留纹理增量、使用后续帧的最终布局。
    for _ in 0..2 {
        full.append(ctx.run_ui(input.clone(), |ui| {
            ui::draw(
                ui,
                ed,
                Viewport::new(Pos::ZERO, 1.0),
                Some(ui::Background {
                    snapshot: shot,
                    texture: &texture,
                }),
                ui::shortcut_hint("F1", "⌘⇧2"),
            );
        }));
    }
    export::rasterize(gfx, &ctx, full, 1100, 760)?
        .save(path)
        .context("保存验收界面截图")?;
    Ok(())
}

pub fn verify_color_ui(gfx: &GpuContext, dir: &std::path::Path) -> Result<()> {
    use crate::color_picker::ColorFormat;
    let mut image = image::RgbaImage::from_fn(1100, 760, |x, y| {
        image::Rgba([(x % 255) as u8, (y % 255) as u8, 180, 255])
    });
    image.put_pixel(400, 300, image::Rgba([18, 52, 86, 255]));
    image.put_pixel(401, 300, image::Rgba([240, 230, 20, 255]));
    let shot = Snapshot {
        origin: Pos::ZERO,
        image,
    };
    let mut ed = Editor::new();
    ed.reset_session(shot.bounds(), false);
    ed.set_selection(Some(Rect::new(160.0, 120.0, 760.0, 450.0)));
    for (format, name) in [
        (ColorFormat::Hex, "color-hex.png"),
        (ColorFormat::Rgb, "color-rgb.png"),
    ] {
        ed.color_format = format;
        let path = dir.join(name);
        render_ui_with_input(
            gfx,
            &shot,
            &mut ed,
            &path,
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::ALT),
                    egui::Event::PointerMoved(egui::pos2(400.4, 300.8)),
                ],
                ..Default::default()
            },
        )?;
        // 放大镜中央像素必须为读数对应的原图颜色，不能选到旁边的黄色像素。
        let rendered = image::open(path)?.to_rgba8();
        anyhow::ensure!(
            same_pixel(rendered.get_pixel(514, 412), shot.image.get_pixel(400, 300)),
            "放大镜中心与颜色采样不一致"
        );
    }
    Ok(())
}

/// 实际 GPU 像素验收：矩形打码、精确笔刷遮罩、局部擦除、撤销重做及负屏幕坐标。
pub fn verify_effects(gfx: &GpuContext, dir: &std::path::Path) -> Result<()> {
    let origin = Pos::new(-400.0, -240.0);
    let shot = Snapshot {
        origin,
        image: image::RgbaImage::from_fn(180, 140, |x, y| {
            image::Rgba(if (x + y) % 2 == 0 {
                [10, 30, 210, 255]
            } else {
                [230, 210, 20, 255]
            })
        }),
    };
    let p = |x, y| origin + Pos::new(x, y);
    let mut ed = Editor::new();
    ed.set_selection(Some(Rect::new(origin.x, origin.y, 180.0, 140.0)));
    ed.set_tool(Tool::Mosaic);
    ed.begin(p(110.0, 96.0));
    ed.extend(p(80.0, 45.0));
    ed.finish(p(40.0, 30.0));
    let mosaic = export::compose(gfx, &shot, &ed.doc)?;
    anyhow::ensure!(
        mosaic.get_pixel(80, 60) != shot.image.get_pixel(80, 60),
        "框选马赛克无效"
    );
    anyhow::ensure!(
        mosaic.get_pixel(80, 60) == mosaic.get_pixel(81, 60),
        "马赛克没有生成色块"
    );
    anyhow::ensure!(
        same_pixel(mosaic.get_pixel(20, 20), shot.image.get_pixel(20, 20)),
        "打码改变了区域外像素"
    );
    ed.set_tool(Tool::Eraser);
    ed.brush_width = 12.0;
    ed.begin(p(25.0, 60.0));
    ed.finish(p(145.0, 60.0));
    let erased = export::compose(gfx, &shot, &ed.doc)?;
    erased.save(dir.join("mosaic-erased.png"))?;
    for x in 30..140 {
        anyhow::ensure!(
            same_pixel(erased.get_pixel(x, 60), shot.image.get_pixel(x, 60)),
            "快速擦除漏掉中间像素 x={x}: 得到 {:?}, 期望 {:?}",
            erased.get_pixel(x, 60),
            shot.image.get_pixel(x, 60)
        );
    }
    anyhow::ensure!(
        erased.get_pixel(80, 40) == mosaic.get_pixel(80, 40),
        "橡皮擦删除了整个马赛克"
    );
    anyhow::ensure!(ed.undo(), "橡皮擦不可撤销");
    anyhow::ensure!(
        export::compose(gfx, &shot, &ed.doc)? == mosaic,
        "撤销没有恢复打码"
    );
    anyhow::ensure!(ed.redo(), "橡皮擦不可重做");
    anyhow::ensure!(
        export::compose(gfx, &shot, &ed.doc)? == erased,
        "重做与擦除像素不一致"
    );
    erased.save(dir.join("mosaic-erased.png"))?;

    ed.begin(p(80.0, 40.0));
    ed.finish(p(80.0, 40.0));
    let clicked = export::compose(gfx, &shot, &ed.doc)?;
    anyhow::ensure!(
        same_pixel(clicked.get_pixel(80, 40), shot.image.get_pixel(80, 40)),
        "单击橡皮擦没有生效"
    );
    anyhow::ensure!(
        clicked.get_pixel(100, 40) == mosaic.get_pixel(100, 40),
        "单击擦除超出笔刷范围"
    );

    // L 形涂抹：包围盒内、轨迹外的像素必须保留原图。
    let mut ed = Editor::new();
    ed.set_selection(Some(Rect::new(origin.x, origin.y, 180.0, 140.0)));
    ed.mosaic_brush = true;
    ed.brush_width = 16.0;
    ed.set_tool(Tool::Mosaic);
    ed.begin(p(30.0, 30.0));
    ed.extend(p(125.0, 30.0));
    ed.finish(p(125.0, 110.0));
    let painted = export::compose(gfx, &shot, &ed.doc)?;
    anyhow::ensure!(
        painted.get_pixel(80, 30) != shot.image.get_pixel(80, 30),
        "涂抹马赛克未生效"
    );
    anyhow::ensure!(
        painted.get_pixel(125, 75) != shot.image.get_pixel(125, 75),
        "马赛克末段漏画"
    );
    anyhow::ensure!(
        same_pixel(painted.get_pixel(80, 75), shot.image.get_pixel(80, 75)),
        "马赛克用包围盒代替了笔刷遮罩"
    );
    painted.save(dir.join("mosaic-brush.png"))?;
    Ok(())
}
