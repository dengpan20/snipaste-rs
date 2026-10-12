//! 复制、保存与贴图共用一次合成，复用预览中的 egui 图元与字体，导出不包含工具条或选区框。
use crate::gfx::{GpuContext, SurfaceWindow};
use anyhow::{Context as _, Result};
use snipaste_capture::Snapshot;
use snipaste_core::Document;
use snipaste_render::{canvas, fonts, Viewport};

#[cfg(debug_assertions)]
pub fn compose(gfx: &GpuContext, snapshot: &Snapshot, doc: &Document) -> Result<image::RgbaImage> {
    compose_with_texture(gfx, snapshot, doc, None)
}

pub fn compose_with_texture(
    gfx: &GpuContext,
    snapshot: &Snapshot,
    doc: &Document,
    background: Option<&wgpu::Texture>,
) -> Result<image::RgbaImage> {
    let selection = doc.selection.context("请先选择截图区域")?;
    if doc.items.is_empty() {
        return Ok(snapshot.crop(selection)?.image);
    }
    let bounds = snapshot.crop_bounds(selection)?;
    let ctx = egui::Context::default();
    if doc
        .items
        .iter()
        .any(|item| matches!(item, snipaste_core::Annotation::Text { .. }))
    {
        fonts::install(&ctx);
    }
    ctx.set_pixels_per_point(1.0);
    ctx.input_mut(|input| {
        input.max_texture_side = gfx.device.limits().max_texture_dimension_2d as usize
    });
    let mut renderer = offscreen_renderer(gfx);
    let loaded = background.is_none().then(|| {
        ctx.load_texture(
            "export.background",
            color_image(&snapshot.image),
            egui::TextureOptions::NEAREST,
        )
    });
    // 正常截图导出复用已上传的底图，不再创建整屏 CPU/GPU 副本。
    let texture = if let Some(background) = background {
        renderer.register_native_texture(
            &gfx.device,
            &background.create_view(&Default::default()),
            wgpu::FilterMode::Nearest,
        )
    } else {
        loaded.as_ref().unwrap().id()
    };
    let size = egui::vec2(bounds.width(), bounds.height());
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let raw = egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
    };
    let full = ctx.run_ui(raw, |ui| {
        let painter = ui.painter().with_clip_rect(rect);
        painter.image(
            texture,
            Viewport::new(bounds.min, 1.0).rect_to_points(&snipaste_core::Rect::new(
                snapshot.origin.x,
                snapshot.origin.y,
                snapshot.image.width() as f32,
                snapshot.image.height() as f32,
            )),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        canvas::paint_document_with_image(
            &painter,
            doc,
            &Viewport::new(bounds.min, 1.0),
            canvas::SourceImage {
                image: &snapshot.image,
                origin: snapshot.origin,
                texture,
            },
        );
    });
    rasterize_with_renderer(
        gfx,
        &ctx,
        full,
        bounds.width() as u32,
        bounds.height() as u32,
        renderer,
    )
}

pub fn color_image(image: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// 离屏渲染读回，物理像素 1:1，不受桌面 DPI 缩放影响。
#[cfg(debug_assertions)]
pub fn rasterize(
    gfx: &GpuContext,
    ctx: &egui::Context,
    full: egui::FullOutput,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage> {
    rasterize_with_renderer(gfx, ctx, full, width, height, offscreen_renderer(gfx))
}

fn offscreen_renderer(gfx: &GpuContext) -> egui_wgpu::Renderer {
    egui_wgpu::Renderer::new(
        &gfx.device,
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::default(),
    )
}

fn rasterize_with_renderer(
    gfx: &GpuContext,
    ctx: &egui::Context,
    mut full: egui::FullOutput,
    width: u32,
    height: u32,
    mut renderer: egui_wgpu::Renderer,
) -> Result<image::RgbaImage> {
    let mut textures = std::mem::take(&mut full.textures_delta);
    for (id, deltas) in &textures.set {
        for delta in deltas {
            renderer.update_texture(&gfx.device, &gfx.queue, *id, delta);
        }
    }
    textures.clear();
    let jobs = ctx.tessellate(full.shapes, full.pixels_per_point);
    let desc = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point: full.pixels_per_point,
    };
    rasterize_prepared(
        gfx,
        &mut renderer,
        &jobs,
        &desc,
        wgpu::TextureFormat::Rgba8Unorm,
    )
}

/// Render with the window's existing textures, without acquiring a hidden Metal drawable.
pub fn rasterize_prepared(
    gfx: &GpuContext,
    renderer: &mut egui_wgpu::Renderer,
    jobs: &[egui::ClippedPrimitive],
    desc: &egui_wgpu::ScreenDescriptor,
    format: wgpu::TextureFormat,
) -> Result<image::RgbaImage> {
    anyhow::ensure!(
        matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
        ),
        "离屏渲染需要 RGBA / BGRA 8 位表面"
    );
    let [width, height] = desc.size_in_pixels;
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let target = gfx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("export"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("export.readback"),
        size: stride as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gfx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("export"),
        });
    let commands = renderer.update_buffers(&gfx.device, &gfx.queue, &mut encoder, jobs, desc);
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("export"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.render(&mut pass.forget_lifetime(), jobs, desc);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        size,
    );
    gfx.queue.submit(
        commands
            .into_iter()
            .chain(std::iter::once(encoder.finish())),
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    gfx.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(10)),
    })?;
    receiver.recv_timeout(std::time::Duration::from_secs(10))??;
    let mapped = buffer.slice(..).get_mapped_range()?;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in mapped.chunks_exact(stride as usize) {
        rgba.extend_from_slice(&row[..width as usize * 4]);
    }
    drop(mapped);
    buffer.unmap();
    if format == wgpu::TextureFormat::Bgra8Unorm {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    image::RgbaImage::from_raw(width, height, rgba).context("合成图片尺寸错误")
}

/// 贴图窗口使用同一套 GPU 呈现流程，每个窗口单独维护纹理与输入上下文。
pub fn present(
    gfx: &GpuContext,
    window: &mut SurfaceWindow,
    ctx: &egui::Context,
    mut full: egui::FullOutput,
) {
    if full
        .viewport_output
        .values()
        .any(|v| v.repaint_delay.is_zero())
    {
        window.window.request_redraw();
    }
    window
        .egui
        .handle_platform_output(&window.window, full.platform_output);
    let mut textures = std::mem::take(&mut full.textures_delta);
    for (id, deltas) in &textures.set {
        for delta in deltas {
            window
                .painter
                .update_texture(&gfx.device, &gfx.queue, *id, delta);
        }
    }
    let jobs = ctx.tessellate(full.shapes, full.pixels_per_point);
    let size = window.size();
    let desc = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [size.width, size.height],
        pixels_per_point: full.pixels_per_point,
    };
    let mut encoder = gfx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pin") });
    let commands =
        window
            .painter
            .update_buffers(&gfx.device, &gfx.queue, &mut encoder, &jobs, &desc);
    let frame = match window.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
            Some(f)
        }
        _ => {
            window.surface.configure(&gfx.device, &window.config);
            None
        }
    };
    if let Some(frame) = frame {
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pin"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            window
                .painter
                .render(&mut pass.forget_lifetime(), &jobs, &desc);
        }
        gfx.queue.submit(
            commands
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        gfx.queue.present(frame);
    }
    for id in &textures.free {
        window.painter.free_texture(id);
    }
    textures.clear();
}
