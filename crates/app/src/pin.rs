//! 桌面贴图：独立无边框置顶窗口，拖动、滚轮缩放、Shift+滚轮透明度、复制和关闭。
use crate::{
    export,
    gfx::{GpuContext, SurfaceWindow},
};
use anyhow::Result;
use snipaste_core::Pos;
use snipaste_render::{fonts, theme, Viewport};
use std::sync::Arc;
use winit::{
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowLevel},
};

#[derive(Clone, Copy)]
pub enum PinCommand {
    Close,
    Copy,
    Save,
}
pub struct PinWindow {
    pub surface: SurfaceWindow,
    pub image: image::RgbaImage,
    ctx: egui::Context,
    texture: egui::TextureHandle,
    zoom: f32,
    opacity: f32,
}
impl PinWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        gfx: &GpuContext,
        image: image::RgbaImage,
        origin: Pos,
    ) -> Result<Self> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        fonts::install(&ctx);
        let attrs = Window::default_attributes()
            .with_title("Snipaste-RS 贴图")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(winit::dpi::PhysicalSize::new(image.width(), image.height()))
            .with_position(winit::dpi::PhysicalPosition::new(
                origin.x as i32,
                origin.y as i32,
            ));
        let window = Arc::new(event_loop.create_window(attrs)?);
        #[cfg(target_os = "macos")]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle() {
                if let RawWindowHandle::AppKit(handle) = handle.as_raw() {
                    unsafe {
                        snipaste_platform::macos::configure_window(handle.ns_view.as_ptr(), false)
                    };
                }
            }
            // The capture origin is expressed using its display's backing scale.
            if let Some(display) = snipaste_platform::macos::capture_display() {
                window.set_outer_position(winit::dpi::LogicalPosition::new(
                    origin.x as f64 / display.scale,
                    origin.y as f64 / display.scale,
                ));
            }
            // Set physical size after moving to the target screen; winit may have
            // initially selected the main screen's backing scale during creation.
            let _ = window
                .request_inner_size(winit::dpi::PhysicalSize::new(image.width(), image.height()));
        }
        let scale = window.scale_factor() as f32;
        let surface = gfx.instance.create_surface(window.clone())?;
        let surface = SurfaceWindow::new(
            gfx,
            &ctx,
            window,
            surface,
            Viewport::new(Pos::ZERO, scale),
            true,
        )?;
        let texture = ctx.load_texture(
            "pin.image",
            export::color_image(&image),
            egui::TextureOptions::LINEAR,
        );
        surface.window.set_visible(true);
        surface.window.request_redraw();
        Ok(Self {
            surface,
            image,
            ctx,
            texture,
            zoom: 1.0,
            opacity: 1.0,
        })
    }
    pub fn event(&mut self, gfx: &GpuContext, event: &WindowEvent) -> Option<PinCommand> {
        let repaint = self
            .surface
            .egui
            .on_window_event(&self.surface.window, event)
            .repaint;
        let command = match event {
            WindowEvent::CloseRequested => Some(PinCommand::Close),
            WindowEvent::Resized(size) => {
                self.surface.resize(gfx, *size);
                None
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.surface.viewport.scale = *scale_factor as f32;
                None
            }
            WindowEvent::RedrawRequested => self.redraw(gfx),
            _ => None,
        };
        if repaint {
            self.surface.window.request_redraw();
        }
        command
    }
    fn redraw(&mut self, gfx: &GpuContext) -> Option<PinCommand> {
        let raw = self.surface.egui.take_egui_input(&self.surface.window);
        let mut command = None;
        let mut drag = false;
        let mut resize = false;
        let full = self.ctx.run_ui(raw, |ui| {
            let rect = ui.max_rect();
            ui.painter().image(
                self.texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::from_white_alpha((self.opacity * 255.0) as u8),
            );
            let response = ui.interact(
                rect,
                egui::Id::new("pin.canvas"),
                egui::Sense::click_and_drag(),
            );
            drag = response.drag_started();
            if response.double_clicked() {
                command = Some(PinCommand::Close);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                command = Some(PinCommand::Close);
            }
            if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C)) {
                command = Some(PinCommand::Copy);
            }
            let (scroll, shift) = ui.input(|i| {
                (
                    i.raw
                        .events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                            _ => None,
                        })
                        .sum::<f32>(),
                    i.modifiers.shift,
                )
            });
            if scroll != 0.0 {
                if shift {
                    self.opacity = (self.opacity + scroll.signum() * 0.05).clamp(0.1, 1.0);
                } else {
                    self.zoom =
                        (self.zoom * if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(0.1, 5.0);
                    resize = true;
                }
            }
            response.context_menu(|ui| {
                if ui
                    .button(crate::ui::shortcut_hint("复制（Ctrl+C）", "复制（⌘C）"))
                    .clicked()
                {
                    command = Some(PinCommand::Copy);
                    ui.close();
                }
                if ui.button("保存…").clicked() {
                    command = Some(PinCommand::Save);
                    ui.close();
                }
                if ui.button("恢复原始大小").clicked() {
                    self.zoom = 1.0;
                    self.opacity = 1.0;
                    resize = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("关闭（Esc / 双击）").clicked() {
                    command = Some(PinCommand::Close);
                    ui.close();
                }
            });
        });
        export::present(gfx, &mut self.surface, &self.ctx, full);
        if drag {
            let _ = self.surface.window.drag_window();
        }
        if resize {
            let _ = self
                .surface
                .window
                .request_inner_size(winit::dpi::PhysicalSize::new(
                    (self.image.width() as f32 * self.zoom).max(32.0) as u32,
                    (self.image.height() as f32 * self.zoom).max(32.0) as u32,
                ));
        }
        command
    }
}
