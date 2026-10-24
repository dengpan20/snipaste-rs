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
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    window::{Window, WindowLevel},
};

// Keep the shadow inside the transparent surface, independent of image zoom.
const SHADOW_MARGIN: f32 = 18.0;

fn shadow_padding(scale: f32) -> u32 {
    (SHADOW_MARGIN * scale).ceil() as u32
}

fn window_size(image: (u32, u32), zoom: f32, scale: f32) -> PhysicalSize<u32> {
    let padding = 2 * shadow_padding(scale);
    PhysicalSize::new(
        (image.0 as f32 * zoom).round().max(1.0) as u32 + padding,
        (image.1 as f32 * zoom).round().max(1.0) as u32 + padding,
    )
}

fn pin_shadow(focused: bool, opacity: f32) -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 2],
        blur: 24,
        spread: 2,
        color: if focused {
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, (128.0 * opacity) as u8)
        } else {
            egui::Color32::from_black_alpha((80.0 * opacity) as u8)
        },
    }
}

fn paint_image(
    ui: &egui::Ui,
    texture: egui::TextureId,
    size: PhysicalSize<u32>,
    scale: f32,
    opacity: f32,
    focused: bool,
) -> egui::Rect {
    // Derive both edges from physical pixels; egui's root layout may round its
    // logical bounds at fractional DPI and subtly stretch the screenshot.
    let padding = shadow_padding(scale) as f32;
    let rect = egui::Rect::from_min_max(
        egui::pos2(padding / scale, padding / scale),
        egui::pos2(
            (size.width as f32 - padding) / scale,
            (size.height as f32 - padding) / scale,
        ),
    );
    ui.painter()
        .add(pin_shadow(focused, opacity).as_shape(rect, 0));
    ui.painter().image(
        texture,
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::from_white_alpha((opacity * 255.0) as u8),
    );
    rect
}

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
    focused: bool,
    fonts_loaded: bool,
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
        let attrs = Window::default_attributes()
            .with_title(format!("{} 贴图", crate::APP_NAME))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(winit::dpi::PhysicalSize::new(image.width(), image.height()))
            .with_position(PhysicalPosition::new(origin.x as i32, origin.y as i32));
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs
                .with_skip_taskbar(true)
                .with_undecorated_shadow(false)
                .with_no_redirection_bitmap(true)
        };
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs.with_has_shadow(false)
        };
        let window = Arc::new(event_loop.create_window(attrs)?);
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle() {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    snipaste_platform::window::configure_pin_window(handle.hwnd.get());
                }
            }
        }
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
                    origin.x as f64 / display.scale - SHADOW_MARGIN as f64,
                    origin.y as f64 / display.scale - SHADOW_MARGIN as f64,
                ));
            }
        }
        let scale = window.scale_factor() as f32;
        #[cfg(not(target_os = "macos"))]
        {
            let padding = shadow_padding(scale) as i32;
            window.set_outer_position(PhysicalPosition::new(
                origin.x as i32 - padding,
                origin.y as i32 - padding,
            ));
        }
        // Padding must not resize the image or move its original screen position.
        // On macOS, move to the target screen before requesting a physical size.
        let _ = window.request_inner_size(window_size(image.dimensions(), 1.0, scale));
        let focused = window.has_focus();
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
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = surface.window.window_handle() {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    // winit rebuilds the native styles when visibility changes.
                    snipaste_platform::window::configure_pin_window(handle.hwnd.get());
                }
            }
        }
        surface.window.request_redraw();
        Ok(Self {
            surface,
            image,
            ctx,
            texture,
            zoom: 1.0,
            opacity: 1.0,
            focused,
            fonts_loaded: false,
        })
    }
    pub fn event(&mut self, gfx: &GpuContext, event: &WindowEvent) -> Option<PinCommand> {
        if !self.fonts_loaded
            && matches!(
                event,
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Right,
                    ..
                }
            )
        {
            // Image-only pins need no CJK font. Load it before the menu's first frame.
            self.fonts_loaded = fonts::install(&self.ctx).is_some();
        }
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
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                inner_size_writer,
            } => {
                self.surface.viewport.scale = *scale_factor as f32;
                // Override winit's suggested logical-size scaling so the image
                // keeps its physical pixels when moved between monitors.
                let _ = inner_size_writer.clone().request_inner_size(window_size(
                    self.image.dimensions(),
                    self.zoom,
                    self.surface.viewport.scale,
                ));
                self.surface.window.request_redraw();
                None
            }
            WindowEvent::Focused(focused) => {
                self.focused = *focused;
                self.surface.window.request_redraw();
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
    #[cfg(debug_assertions)]
    pub fn verify_appearance(
        &mut self,
        gfx: &GpuContext,
        directory: &std::path::Path,
    ) -> Result<()> {
        let original_focus = self.focused;
        for scale in [1.0, 1.25, 2.0] {
            for focused in [false, true] {
                self.event(gfx, &WindowEvent::Focused(focused));
                anyhow::ensure!(self.focused == focused, "贴图焦点状态未更新");
                #[cfg(windows)]
                {
                    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    if let RawWindowHandle::Win32(handle) =
                        self.surface.window.window_handle()?.as_raw()
                    {
                        anyhow::ensure!(
                            snipaste_platform::window::pin_window_hides_taskbar(handle.hwnd.get()),
                            "贴图窗口仍带有任务栏样式"
                        );
                    }
                }
                let ctx = egui::Context::default();
                theme::apply(&ctx);
                let texture = ctx.load_texture(
                    "pin.verify",
                    export::color_image(&self.image),
                    egui::TextureOptions::LINEAR,
                );
                let size = window_size(self.image.dimensions(), 1.0, scale);
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(size.width as f32 / scale, size.height as f32 / scale),
                    )),
                    ..Default::default()
                };
                raw.viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                let full = ctx.run_ui(raw, |ui| {
                    paint_image(ui, texture.id(), size, scale, 1.0, self.focused);
                });
                let rendered = export::rasterize(gfx, &ctx, full, size.width, size.height)?;
                rendered.save(directory.join(format!("pin-shadow-{focused}-{scale}.png")))?;
                let padding = shadow_padding(scale);
                let shadow = rendered.get_pixel(padding - 3, size.height / 2);
                anyhow::ensure!(shadow[3] > 0 && shadow[3] < 255, "贴图阴影没有半透明像素");
                if focused {
                    anyhow::ensure!(
                        shadow[2] > shadow[1] && shadow[1] > shadow[0],
                        "焦点阴影不是浅蓝色"
                    );
                } else {
                    anyhow::ensure!(shadow.0[0..3] == [0, 0, 0], "非焦点阴影不是中性阴影");
                }
                anyhow::ensure!(rendered.get_pixel(0, 0)[3] == 0, "阴影外边距不透明");
                // Verify every original pixel: padding and DPI must not resample the image.
                for (x, y, pixel) in self.image.enumerate_pixels() {
                    let actual = rendered.get_pixel(x + padding, y + padding);
                    anyhow::ensure!(
                        actual
                            .0
                            .iter()
                            .zip(pixel.0)
                            .all(|(a, b)| a.abs_diff(b) <= 1),
                        "贴图内容因阴影边距发生变化 ({x}, {y}) @ {scale}: {actual:?}, expected {pixel:?}"
                    );
                }
            }
        }
        self.event(gfx, &WindowEvent::Focused(original_focus));
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn verify_menu_fonts(&mut self, gfx: &GpuContext) -> Result<()> {
        anyhow::ensure!(!self.fonts_loaded, "贴图提前加载了中文字体");
        self.event(
            gfx,
            &WindowEvent::MouseInput {
                device_id: winit::event::DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Right,
            },
        );
        self.event(gfx, &WindowEvent::RedrawRequested);
        anyhow::ensure!(self.fonts_loaded, "贴图右键菜单未加载中文字体");
        anyhow::ensure!(
            self.ctx.fonts_mut(|fonts| fonts.has_glyphs(
                &egui::FontId::proportional(14.0),
                "复制保存恢复原始大小关闭"
            )),
            "贴图菜单缺少中文字形"
        );
        self.event(
            gfx,
            &WindowEvent::MouseInput {
                device_id: winit::event::DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Right,
            },
        );
        Ok(())
    }

    fn redraw(&mut self, gfx: &GpuContext) -> Option<PinCommand> {
        let raw = self.surface.egui.take_egui_input(&self.surface.window);
        let mut command = None;
        let mut drag = false;
        let mut resize = false;
        let full = self.ctx.run_ui(raw, |ui| {
            let rect = paint_image(
                ui,
                self.texture.id(),
                self.surface.size(),
                self.surface.viewport.scale,
                self.opacity,
                self.focused,
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
                    ui.ctx().request_repaint();
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
            self.resize_image();
        }
        command
    }

    fn resize_image(&self) {
        let _ = self.surface.window.request_inner_size(window_size(
            self.image.dimensions(),
            self.zoom,
            self.surface.viewport.scale,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_padding_keeps_original_pixels_at_every_dpi() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let size = window_size((760, 450), 1.0, scale);
            let padding = 2 * shadow_padding(scale);
            assert_eq!((size.width - padding, size.height - padding), (760, 450));
            let zoomed = window_size((760, 450), 0.5, scale);
            assert_eq!(
                (zoomed.width - padding, zoomed.height - padding),
                (380, 225)
            );
        }
        let tiny = window_size((8, 4), 1.0, 1.0);
        assert_eq!((tiny.width - 36, tiny.height - 36), (8, 4));
    }

    #[test]
    fn shadow_fits_inside_margin_in_both_focus_states() {
        for focused in [false, true] {
            let shadow = pin_shadow(focused, 1.0);
            let margin = shadow.margin();
            for extent in [margin.left, margin.right, margin.top, margin.bottom] {
                assert!(extent < SHADOW_MARGIN, "shadow would be clipped");
            }
            assert!(pin_shadow(focused, 0.5).color.a() < shadow.color.a());
        }
    }
}
