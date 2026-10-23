//! On-demand settings window and small, atomically replaced user preferences.
use crate::{
    color_picker::ColorFormat,
    editor::Editor,
    export,
    gfx::{GpuContext, SurfaceWindow},
    hotkey::{self, Action},
};
use anyhow::{Context as _, Result};
use global_hotkey::hotkey::{HotKey, Modifiers};
use serde::{Deserialize, Serialize};
use snipaste_core::{Color, MosaicMode, Pos};
use snipaste_render::{fonts, theme, Viewport};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use winit::{
    event::{ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::Window,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub capture_hotkey: String,
    pub paste_hotkey: String,
    pub save_directory: String,
    pub image_format: ImageFormat,
    pub window_snapping: bool,
    pub remember_annotation: bool,
    pub annotation: AnnotationPreferences,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            capture_hotkey: String::new(),
            paste_hotkey: String::new(),
            save_directory: String::new(),
            image_format: ImageFormat::Png,
            window_snapping: true,
            remember_annotation: true,
            annotation: AnnotationPreferences::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnnotationPreferences {
    pub color: [u8; 4],
    pub stroke_width: f32,
    pub font_size: f32,
    pub sequence_size: f32,
    pub brush_width: f32,
    pub mosaic_block: u32,
    pub mosaic_blur: bool,
    pub filled: bool,
    pub mosaic_brush: bool,
    pub eraser_brush: bool,
    pub rgb_color: bool,
}

impl Default for AnnotationPreferences {
    fn default() -> Self {
        Self::from_editor(&Editor::new())
    }
}

impl AnnotationPreferences {
    pub fn from_editor(ed: &Editor) -> Self {
        let c = ed.style.stroke;
        Self {
            color: [c.r, c.g, c.b, c.a],
            stroke_width: ed.style.stroke_width,
            font_size: ed.style.font_size,
            sequence_size: ed.sequence_size,
            brush_width: ed.brush_width,
            mosaic_block: ed.style.mosaic_block,
            mosaic_blur: ed.style.mosaic_mode == MosaicMode::Blur,
            filled: ed.style.fill.is_some(),
            mosaic_brush: ed.mosaic_brush,
            eraser_brush: ed.eraser_brush,
            rgb_color: ed.color_format == ColorFormat::Rgb,
        }
    }

    pub fn apply(&self, ed: &mut Editor) {
        ed.style.stroke = Color::rgba(self.color[0], self.color[1], self.color[2], self.color[3]);
        ed.style.stroke_width = self.stroke_width;
        ed.style.font_size = self.font_size;
        ed.style.fill = self.filled.then_some(ed.style.stroke);
        ed.style.mosaic_block = self.mosaic_block;
        ed.style.mosaic_mode = if self.mosaic_blur {
            MosaicMode::Blur
        } else {
            MosaicMode::Pixelate
        };
        ed.sequence_size = self.sequence_size;
        ed.brush_width = self.brush_width;
        ed.mosaic_brush = self.mosaic_brush;
        ed.eraser_brush = self.eraser_brush;
        ed.color_format = if self.rgb_color {
            ColorFormat::Rgb
        } else {
            ColorFormat::Hex
        };
    }
}

impl Preferences {
    pub fn validate(&self) -> Result<()> {
        crate::hotkey::validate_pair(&self.capture_hotkey, &self.paste_hotkey)?;
        for (name, value, min, max) in [
            ("笔宽", self.annotation.stroke_width, 1.0, 32.0),
            ("字号", self.annotation.font_size, 10.0, 96.0),
            ("序号大小", self.annotation.sequence_size, 20.0, 96.0),
            ("笔刷大小", self.annotation.brush_width, 4.0, 96.0),
        ] {
            anyhow::ensure!(
                value.is_finite() && (min..=max).contains(&value),
                "{name}需在 {min}–{max} 之间"
            );
        }
        anyhow::ensure!(
            (4..=40).contains(&self.annotation.mosaic_block),
            "马赛克强度需在 4–40 之间"
        );
        Ok(())
    }

    pub fn validate_directory(&self) -> Result<()> {
        if !self.save_directory.trim().is_empty() {
            let path = Path::new(self.save_directory.trim());
            anyhow::ensure!(
                path.is_absolute() && path.is_dir(),
                "保存目录需为已存在的文件夹，请填写完整路径"
            );
        }
        Ok(())
    }
}

pub struct SettingsStore {
    path: PathBuf,
    migrate_legacy: bool,
}

impl SettingsStore {
    pub fn for_user() -> Result<Self> {
        if let Some(path) = std::env::var_os("SNIPASTE_SETTINGS_PATH") {
            return Ok(Self {
                path: PathBuf::from(path),
                migrate_legacy: false,
            });
        }
        if let Some(directory) = std::env::var_os("SNIPASTE_VERIFY_SETTINGS_DIR")
            .or_else(|| std::env::var_os("SNIPASTE_VERIFY_DIR"))
            .or_else(|| std::env::var_os("SNIPASTE_MEMORY_PROFILE_DIR"))
        {
            return Ok(Self {
                path: PathBuf::from(directory).join("settings.toml"),
                migrate_legacy: false,
            });
        }
        #[cfg(windows)]
        let root = PathBuf::from(std::env::var_os("APPDATA").context("无法获取 APPDATA 设置目录")?);
        #[cfg(not(windows))]
        let root = PathBuf::from(std::env::var_os("HOME").context("无法获取用户设置目录")?)
            .join("Library/Application Support");
        Ok(Self {
            path: root.join("Yingtie/settings.toml"),
            migrate_legacy: true,
        })
    }

    pub fn load(&self) -> Result<Preferences> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let prefs: Preferences = toml::from_str(&text).context("设置文件格式无效")?;
                prefs.validate()?;
                Ok(prefs)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut prefs = Preferences::default();
                if self.migrate_legacy {
                    let legacy = hotkey::Config::load().context("读取旧版热键设置失败")?;
                    prefs.capture_hotkey = legacy.capture.unwrap_or_default();
                    prefs.paste_hotkey = legacy.paste.unwrap_or_default();
                }
                Ok(prefs)
            }
            Err(e) => Err(e).context("无法读取设置文件"),
        }
    }

    pub fn save(&self, preferences: &Preferences) -> Result<()> {
        preferences.validate()?;
        let contents = toml::to_string_pretty(preferences)?;
        let parent = self.path.parent().context("设置目录无效")?;
        std::fs::create_dir_all(parent).context("无法创建设置目录")?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = std::fs::File::create(&temporary).context("无法写入设置文件")?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path).context("无法替换设置文件")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

pub enum SettingsCommand {
    Close,
    Save,
    BrowseDirectory,
    Record(Action),
    RestoreDefaults,
}

pub struct SettingsWindow {
    pub surface: SurfaceWindow,
    ctx: egui::Context,
    pub draft: Preferences,
    pub autostart: bool,
    pub error: Option<String>,
    pub repaint_deadline: Option<Instant>,
    hotkey_labels: (String, String),
    recording: Option<Action>,
    modifiers: ModifiersState,
}

impl SettingsWindow {
    #[cfg(debug_assertions)]
    pub fn verify_fonts(&self) -> Result<()> {
        anyhow::ensure!(
            self.ctx.fonts_mut(|f| f.has_glyphs(
                &egui::FontId::proportional(14.0),
                "设置快捷键截图桌面贴图默认保存目录标注偏好序号大小恢复默认取消保存设置"
            )),
            "设置窗口缺少中文字形"
        );
        Ok(())
    }
    pub fn new(
        event_loop: &ActiveEventLoop,
        gfx: &mut Option<GpuContext>,
        draft: Preferences,
        autostart: bool,
        hotkey_labels: (String, String),
    ) -> Result<Self> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        fonts::install(&ctx);
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title(format!("{} 设置", crate::APP_NAME))
                    .with_inner_size(winit::dpi::LogicalSize::new(600.0, 650.0))
                    .with_min_inner_size(winit::dpi::LogicalSize::new(490.0, 430.0))
                    .with_visible(false),
            )?,
        );
        let surface = if let Some(gfx) = gfx.as_ref() {
            gfx.instance.create_surface(window.clone())?
        } else {
            let (created, surface) = pollster::block_on(GpuContext::new(&window))?;
            *gfx = Some(created);
            surface
        };
        let scale = window.scale_factor() as f32;
        let surface = SurfaceWindow::new(
            gfx.as_ref().unwrap(),
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
            ctx,
            draft,
            autostart,
            error: None,
            repaint_deadline: None,
            hotkey_labels,
            recording: None,
            modifiers: ModifiersState::empty(),
        })
    }

    pub fn event(&mut self, gfx: &GpuContext, event: &WindowEvent) -> Option<SettingsCommand> {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) => {
                self.modifiers = ModifiersState::empty();
                self.recording = None;
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && self.recording.is_some() =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.record_key(code);
                    self.surface.window.request_redraw();
                    return None;
                }
            }
            _ => {}
        }
        let repaint = self
            .surface
            .egui
            .on_window_event(&self.surface.window, event)
            .repaint;
        let command = match event {
            WindowEvent::CloseRequested => Some(SettingsCommand::Close),
            WindowEvent::Resized(size) => {
                self.surface.resize(gfx, *size);
                None
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.surface.viewport.scale = *scale_factor as f32;
                None
            }
            WindowEvent::RedrawRequested
                if self.surface.size().width > 0 && self.surface.size().height > 0 =>
            {
                self.redraw(gfx)
            }
            _ => None,
        };
        if repaint {
            self.surface.window.request_redraw();
        }
        command
    }

    pub fn start_recording(&mut self, action: Action) {
        self.recording = Some(action);
        self.error = None;
        self.surface.window.request_redraw();
    }

    pub fn restore_defaults(&mut self) {
        self.draft = Preferences::default();
        self.autostart = false;
        self.recording = None;
        self.error = None;
        self.surface.window.request_redraw();
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
                | KeyCode::AltLeft
                | KeyCode::AltRight
                | KeyCode::ShiftLeft
                | KeyCode::ShiftRight
                | KeyCode::SuperLeft
                | KeyCode::SuperRight
        ) {
            return;
        }
        match recorded_key(code, self.modifiers) {
            Ok(key) => {
                let mut config =
                    hotkey::config_from_text(&self.draft.capture_hotkey, &self.draft.paste_hotkey);
                config.set(self.recording.take().unwrap(), key);
                self.draft.capture_hotkey = config.capture.unwrap_or_default();
                self.draft.paste_hotkey = config.paste.unwrap_or_default();
                self.error = None;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn redraw(&mut self, gfx: &GpuContext) -> Option<SettingsCommand> {
        let input = self.surface.egui.take_egui_input(&self.surface.window);
        let mut command = None;
        let full = self.ctx.run_ui(input, |ui| {
            command = panel(
                ui,
                &mut self.draft,
                &mut self.autostart,
                self.error.as_deref(),
                &self.hotkey_labels,
                self.recording,
            );
        });
        self.repaint_deadline = full
            .viewport_output
            .values()
            .filter_map(|v| Instant::now().checked_add(v.repaint_delay))
            .min();
        export::present(gfx, &mut self.surface, &self.ctx, full);
        command
    }
}

fn panel(
    ui: &mut egui::Ui,
    prefs: &mut Preferences,
    autostart: &mut bool,
    error: Option<&str>,
    labels: &(String, String),
    recording: Option<Action>,
) -> Option<SettingsCommand> {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, theme::TOOLBAR_BG);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(16.0)), |ui| {
        panel_contents(ui, prefs, autostart, error, labels, recording)
    })
    .inner
}

fn panel_contents(
    ui: &mut egui::Ui,
    prefs: &mut Preferences,
    autostart: &mut bool,
    error: Option<&str>,
    labels: &(String, String),
    recording: Option<Action>,
) -> Option<SettingsCommand> {
    let mut command = None;
    let height = ui.available_height();
    ui.vertical(|ui| {
        ui.heading("设置");
        ui.label("保存后立即生效，关闭窗口会放弃未保存的修改。");
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height((height - 120.0).max(180.0))
            .show(ui, |ui| {
                ui.strong("快捷键");
                for action in Action::ALL {
                    ui.horizontal(|ui| {
                        ui.add_sized([64.0, 28.0], egui::Label::new(action.label()));
                        let value = match action {
                            Action::Capture => &mut prefs.capture_hotkey,
                            Action::PasteLast => &mut prefs.paste_hotkey,
                        };
                        ui.add_sized(
                            [250.0, 28.0],
                            egui::TextEdit::singleline(value)
                                .hint_text("留空使用默认快捷键")
                                .desired_width(250.0),
                        );
                        if ui
                            .button(if recording == Some(action) {
                                "请按键…"
                            } else {
                                "录入"
                            })
                            .clicked()
                        {
                            command = Some(SettingsCommand::Record(action));
                        }
                    });
                }
                ui.small(if cfg!(target_os = "macos") {
                    "示例：Command+Shift+2、Command+Shift+1、F1"
                } else {
                    "示例：Ctrl+Shift+S、Alt+F3、F1"
                });
                ui.small(format!("当前生效：截图 {}；贴图 {}", labels.0, labels.1));
                ui.small("点击录入后按下组合键，Esc 取消录入；设置窗口内暂停全局热键。");
                ui.add_space(10.0);
                ui.separator();
                ui.strong("截图与保存");
                ui.checkbox(autostart, "登录后自动启动");
                ui.checkbox(&mut prefs.window_snapping, "截图时自动预选鼠标下的窗口");
                ui.horizontal(|ui| {
                    ui.label("默认格式");
                    ui.selectable_value(&mut prefs.image_format, ImageFormat::Png, "PNG");
                    ui.selectable_value(&mut prefs.image_format, ImageFormat::Jpeg, "JPEG");
                });
                ui.label("默认保存目录");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut prefs.save_directory)
                            .desired_width((ui.available_width() - 76.0).max(200.0))
                            .hint_text("留空使用系统默认目录"),
                    );
                    if ui.button("选择…").clicked() {
                        command = Some(SettingsCommand::BrowseDirectory);
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.strong("标注偏好");
                ui.checkbox(
                    &mut prefs.remember_annotation,
                    "记住上次使用的颜色、大小和笔刷偏好",
                );
                ui.horizontal(|ui| {
                    ui.label("颜色");
                    ui.color_edit_button_srgba_unmultiplied(&mut prefs.annotation.color);
                    ui.label("笔宽");
                    ui.add(
                        egui::DragValue::new(&mut prefs.annotation.stroke_width)
                            .range(1.0..=32.0)
                            .suffix(" px"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("文字字号");
                    ui.add(
                        egui::DragValue::new(&mut prefs.annotation.font_size)
                            .range(10.0..=96.0)
                            .suffix(" px"),
                    );
                    ui.label("序号大小");
                    ui.add(
                        egui::DragValue::new(&mut prefs.annotation.sequence_size)
                            .range(20.0..=96.0)
                            .suffix(" px"),
                    );
                });
                ui.small("序号工具：按 N 后点击添加，切换指针 V 可拖动，Delete 删除。");
            });
        ui.add_space(6.0);
        if let Some(error) = error {
            ui.colored_label(egui::Color32::from_rgb(185, 40, 40), error);
        }
        ui.horizontal(|ui| {
            if ui.button("恢复默认").clicked() {
                command = Some(SettingsCommand::RestoreDefaults);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(recording.is_none(), egui::Button::new("保存设置"))
                    .clicked()
                {
                    command = Some(SettingsCommand::Save);
                }
                if ui.button("取消").clicked() {
                    command = Some(SettingsCommand::Close);
                }
            });
        });
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        command = Some(SettingsCommand::Close);
    }
    command
}

#[cfg(debug_assertions)]
pub fn render_preview(gfx: &GpuContext, directory: &Path) -> Result<()> {
    let mut prefs = Preferences::default();
    let labels = if cfg!(target_os = "macos") {
        ("⌘⇧2 / F1".into(), "⌘⇧1 / F3".into())
    } else {
        ("F1".into(), "F3".into())
    };
    for (width, height, filename, error) in [
        (600, 650, "settings-ui.png", None),
        (
            490,
            430,
            "settings-small-error-ui.png",
            Some("截图快捷键已被占用，请更换后重试。"),
        ),
    ] {
        // Each offscreen renderer starts without textures; it needs a fresh atlas.
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        fonts::install(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            ..Default::default()
        };
        let mut full = ctx.run_ui(input.clone(), |ui| {
            panel(ui, &mut prefs, &mut false, error, &labels, None);
        });
        for _ in 0..2 {
            full.append(ctx.run_ui(input.clone(), |ui| {
                panel(ui, &mut prefs, &mut false, error, &labels, None);
            }));
        }
        export::rasterize(gfx, &ctx, full, width, height)?.save(directory.join(filename))?;
    }
    Ok(())
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

    #[test]
    fn settings_round_trip_and_editor_preferences_survive_restart() {
        let dir = std::env::temp_dir().join(format!("yingtie-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SettingsStore {
            path: dir.join("设置.toml"),
            migrate_legacy: false,
        };
        let mut prefs = Preferences {
            capture_hotkey: "Ctrl+Shift+F8".into(),
            ..Default::default()
        };
        let mut ed = Editor::new();
        ed.sequence_size = 54.0;
        ed.style.stroke = Color::rgb(5, 90, 200);
        prefs.annotation = AnnotationPreferences::from_editor(&ed);
        store.save(&prefs).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded, prefs);
        let mut restarted = Editor::new();
        loaded.annotation.apply(&mut restarted);
        assert_eq!(restarted.sequence_size, 54.0);
        assert_eq!(restarted.style.stroke, ed.style.stroke);
        prefs.image_format = ImageFormat::Jpeg;
        store.save(&prefs).unwrap();
        assert_eq!(store.load().unwrap().image_format, ImageFormat::Jpeg);
        let invalid = Preferences {
            paste_hotkey: prefs.capture_hotkey.clone(),
            ..prefs
        };
        assert!(store.save(&invalid).is_err());
        assert_eq!(store.load().unwrap().image_format, ImageFormat::Jpeg);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn older_settings_get_defaults_and_invalid_sizes_are_rejected() {
        let prefs: Preferences = toml::from_str("window_snapping = false").unwrap();
        assert!(!prefs.window_snapping);
        assert_eq!(prefs.annotation.sequence_size, 32.0);
        let mut invalid = prefs;
        invalid.annotation.sequence_size = f32::NAN;
        assert!(invalid.validate().is_err());
    }
}
