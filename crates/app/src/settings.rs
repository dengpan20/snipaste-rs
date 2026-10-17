//! 独立热键设置窗口，按实际物理键录入，保存前由应用验证系统注册和持久化。
use std::sync::Arc;

use anyhow::Result;
use global_hotkey::hotkey::{HotKey, Modifiers};
use snipaste_core::Pos;
use snipaste_render::{fonts, theme, Viewport};
use winit::{
    event::{ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::Window,
};

use crate::{
    export,
    gfx::{GpuContext, SurfaceWindow},
    hotkey::{self, Action, Config},
};

pub enum Command {
    Save,
    Cancel,
}

pub struct SettingsWindow {
    pub surface: SurfaceWindow,
    pub draft: Config,
    pub error: Option<String>,
    ctx: egui::Context,
    recording: Option<Action>,
    modifiers: ModifiersState,
    active_labels: [String; 2],
}

impl SettingsWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        gfx: &mut Option<GpuContext>,
        draft: Config,
        active_labels: [String; 2],
        error: Option<String>,
    ) -> Result<Self> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        fonts::install(&ctx);
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title(format!("{} · 热键设置", crate::APP_NAME))
                    .with_inner_size(winit::dpi::LogicalSize::new(580.0, 430.0))
                    .with_min_inner_size(winit::dpi::LogicalSize::new(540.0, 400.0))
                    .with_visible(false),
            )?,
        );
        let surface = if let Some(gfx) = gfx.as_ref() {
            gfx.instance.create_surface(window.clone())?
        } else {
            let (context, surface) = pollster::block_on(GpuContext::new(&window))?;
            *gfx = Some(context);
            surface
        };
        let scale = window.scale_factor() as f32;
        let surface = SurfaceWindow::new(
            gfx.as_ref().expect("GPU 已创建"),
            &ctx,
            window,
            surface,
            Viewport::new(Pos::ZERO, scale),
            false,
        )?;
        surface.window.set_visible(true);
        surface.window.focus_window();
        surface.window.request_redraw();
        Ok(Self {
            surface,
            draft,
            error,
            ctx,
            recording: None,
            modifiers: ModifiersState::empty(),
            active_labels,
        })
    }

    pub fn event(&mut self, gfx: &GpuContext, event: &WindowEvent) -> Option<Command> {
        match event {
            WindowEvent::CloseRequested => return Some(Command::Cancel),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) => {
                self.modifiers = ModifiersState::empty();
                self.recording = None;
                self.surface.window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if self.recording.is_some() {
                        self.record_key(code);
                        self.surface.window.request_redraw();
                        return None;
                    }
                    if code == KeyCode::Escape {
                        return Some(Command::Cancel);
                    }
                }
            }
            WindowEvent::Resized(size) => self.surface.resize(gfx, *size),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.surface.viewport.scale = *scale_factor as f32;
            }
            WindowEvent::RedrawRequested => return self.redraw(gfx),
            _ => {}
        }
        if self
            .surface
            .egui
            .on_window_event(&self.surface.window, event)
            .repaint
        {
            self.surface.window.request_redraw();
        }
        None
    }

    fn record_key(&mut self, code: KeyCode) {
        if code == KeyCode::Escape {
            self.recording = None;
            self.error = None;
            return;
        }
        if matches!(
            code,
            KeyCode::ControlLeft
                | KeyCode::ControlRight
                | KeyCode::ShiftLeft
                | KeyCode::ShiftRight
                | KeyCode::AltLeft
                | KeyCode::AltRight
                | KeyCode::SuperLeft
                | KeyCode::SuperRight
        ) {
            return;
        }
        let result = recorded_key(code, self.modifiers);
        match result {
            Ok(key) => {
                if let Some(action) = self.recording.take() {
                    self.draft.set(action, key);
                }
                self.error = None;
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }

    fn redraw(&mut self, gfx: &GpuContext) -> Option<Command> {
        let size = self.surface.size();
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let input = self.surface.egui.take_egui_input(&self.surface.window);
        let mut command = None;
        let full = self.ctx.run_ui(input, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, theme::TOOLBAR_BG);
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.heading("自定义热键");
                        ui.add_space(6.0);
                        ui.label("点击按键框，然后按下新组合。Esc 取消录入。");
                        ui.add_space(12.0);
                        for (index, action) in Action::ALL.into_iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add_sized([96.0, 32.0], egui::Label::new(action.label()));
                                let recording = self.recording == Some(action);
                                let text = if recording {
                                    "请按下热键…".to_owned()
                                } else {
                                    let mut text = self
                                        .draft
                                        .key(action)
                                        .map(hotkey::label)
                                        .unwrap_or_default();
                                    if self.draft.binding(action).is_none() {
                                        text.push_str("（默认）");
                                    }
                                    text
                                };
                                if ui
                                    .add_sized(
                                        [300.0, 36.0],
                                        egui::Button::new(text).selected(recording),
                                    )
                                    .clicked()
                                {
                                    self.recording = Some(action);
                                    self.error = None;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(102.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "当前生效：{}",
                                        self.active_labels[index]
                                    ))
                                    .small()
                                    .weak(),
                                );
                            });
                            ui.add_space(10.0);
                        }
                        ui.label(egui::RichText::new(crate::ui::shortcut_hint(
                        "普通按键请搭配 Ctrl、Alt 或 Win；F1–F24 / PrintScreen 可单独使用。",
                        "普通按键请搭配 Ctrl、Option 或 ⌘；F1–F24 可单独使用。",
                    )).small());
                        ui.label(
                            egui::RichText::new(
                                "此窗口打开期间暂停截图和贴图热键，保存或关闭后恢复。",
                            )
                            .small()
                            .weak(),
                        );
                        ui.add_space(10.0);
                        if let Some(error) = &self.error {
                            ui.colored_label(egui::Color32::from_rgb(190, 45, 45), error);
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("恢复默认").clicked() {
                                self.draft = Config::default();
                                self.recording = None;
                                self.error = None;
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(
                                            self.recording.is_none(),
                                            egui::Button::new("保存"),
                                        )
                                        .clicked()
                                    {
                                        command = Some(Command::Save);
                                    }
                                    if ui.button("取消").clicked() {
                                        command = Some(Command::Cancel);
                                    }
                                },
                            );
                        });
                    });
            });
        });
        export::present(gfx, &mut self.surface, &self.ctx, full);
        command
    }
}

fn recorded_key(code: KeyCode, state: ModifiersState) -> Result<HotKey> {
    // winit 与 keyboard-types 的常用物理键使用相同的 W3C Code 名称。
    let key = format!("{code:?}").parse::<HotKey>()?;
    let mut modifiers = Modifiers::empty();
    for (pressed, modifier) in [
        (state.control_key(), Modifiers::CONTROL),
        (state.alt_key(), Modifiers::ALT),
        (state.shift_key(), Modifiers::SHIFT),
        (state.super_key(), Modifiers::SUPER),
    ] {
        if pressed {
            modifiers.insert(modifier);
        }
    }
    hotkey::parse(&HotKey::new(Some(modifiers), key.key).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_physical_keys_and_modifiers() {
        let key =
            recorded_key(KeyCode::KeyA, ModifiersState::CONTROL | ModifiersState::ALT).unwrap();
        assert_eq!(
            hotkey::label(key),
            crate::ui::shortcut_hint("Ctrl+Alt+A", "Ctrl+Option+A")
        );
        assert!(recorded_key(KeyCode::KeyA, ModifiersState::empty()).is_err());
        assert_eq!(
            recorded_key(KeyCode::F13, ModifiersState::empty())
                .unwrap()
                .key,
            global_hotkey::hotkey::Code::F13
        );
    }
}
