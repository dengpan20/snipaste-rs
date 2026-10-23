//! Snipaste 风格的选区工具条：选定后出现，停靠在选区右下方，空间不足时翻到上方。
use crate::color_picker::{self, ColorFormat};
use crate::editor::{Editor, QrResult};
use egui::{Color32, Context, FontId, Rect as ERect, Stroke, StrokeKind};
use snipaste_core::style::{Tool, PALETTE};
use snipaste_core::Handle;
use snipaste_render::{canvas, theme, Viewport};

pub fn shortcut_hint<'a>(windows: &'a str, macos: &'a str) -> &'a str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        windows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Undo,
    Redo,
    SaveAs,
    Pin,
    CopyAndExit,
    Cancel,
    ClearAll,
    RecognizeQr,
    CopyQr,
    CopyColor { color: [u8; 3], format: ColorFormat },
}

#[derive(Clone, Copy)]
pub struct Background<'a> {
    pub snapshot: &'a snipaste_capture::Snapshot,
    pub texture: &'a egui::TextureHandle,
}

pub fn draw(
    ui: &mut egui::Ui,
    ed: &mut Editor,
    view: Viewport,
    background: Option<Background<'_>>,
    hotkey: &str,
    paste_hotkey: &str,
) -> Vec<Command> {
    let mut cmds = Vec::new();
    let screen = ui.max_rect();
    let ctx = ui.ctx().clone();
    handle_canvas_input(ui, ed, view, screen, &mut cmds);
    paint_canvas(ui.painter(), ed, view, screen, background.as_ref());
    if !ed.is_snapping() && !ed.is_dragging() {
        if let Some(sel) = ed.doc.selection.filter(|s| !s.is_empty()) {
            toolbar(
                &ctx,
                ed,
                screen,
                view.rect_to_points(&sel),
                paste_hotkey,
                &mut cmds,
            );
        }
    }
    text_editor(&ctx, ed, view, screen);
    qr_result(&ctx, ed, &mut cmds);
    if let Some(bg) = background.as_ref() {
        if ed.show_magnifier(ctx.input(|i| i.modifiers.alt)) {
            magnifier(&ctx, bg, view, screen, ed.color_format);
        }
    }
    let hint = ed
        .hint
        .as_deref()
        .unwrap_or(if ed.is_snapping() || ed.doc.selection.is_none() {
            "单击选择窗口 · 拖动框选 · Shift 切换 HEX / RGB · C 复制颜色 · Esc 取消"
        } else {
            "Enter / 双击完成复制 · 方向键微调 · 右键重新选择 · Esc 取消"
        });
    egui::Area::new(egui::Id::new("snipaste.hint"))
        .interactable(false)
        .fixed_pos(egui::pos2(screen.center().x, screen.bottom() - 16.0))
        .pivot(egui::Align2::CENTER_BOTTOM)
        .order(egui::Order::Foreground)
        .show(&ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_black_alpha(205))
                .corner_radius(3.0)
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{hotkey} 截图　{hint}"))
                            .color(Color32::WHITE)
                            .size(12.0),
                    );
                });
        });
    cmds
}

fn paint_canvas(
    painter: &egui::Painter,
    ed: &Editor,
    view: Viewport,
    screen: ERect,
    background: Option<&Background<'_>>,
) {
    if let Some(bg) = background {
        painter.image(
            bg.texture.id(),
            view.rect_to_points(&bg.snapshot.bounds()),
            ERect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let sel_points = ed.doc.selection.map(|r| view.rect_to_points(&r));
    canvas::paint_dim_outside(painter, screen, sel_points);
    let clipped = sel_points
        .filter(|r| r.is_positive())
        .map(|r| painter.with_clip_rect(r));
    let p = clipped.as_ref().unwrap_or(painter);
    if let Some(bg) = background {
        canvas::paint_document_with_image(
            p,
            &ed.doc,
            &view,
            canvas::SourceImage {
                image: &bg.snapshot.image,
                origin: bg.snapshot.origin,
                texture: bg.texture.id(),
            },
        );
    } else {
        canvas::paint_document(p, &ed.doc, &view);
    }
    if let Some(sel) = &ed.doc.selection {
        canvas::paint_selection_frame(painter, sel, &view);
    }
    if let Some(item) = ed.selected_item.and_then(|index| ed.doc.items.get(index)) {
        painter.rect_stroke(
            view.rect_to_points(&item.bounds()),
            0.0,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    }
}

fn handle_canvas_input(
    ui: &mut egui::Ui,
    ed: &mut Editor,
    view: Viewport,
    screen: ERect,
    cmds: &mut Vec<Command>,
) {
    ed.constrain = ui.input(|i| i.modifiers.shift);
    if ed.qr.is_some() {
        return;
    }
    let resp = ui.interact(
        screen,
        egui::Id::new("snipaste.canvas"),
        egui::Sense::click_and_drag(),
    );
    let (pressed, released, down, origin, latest) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_released(egui::PointerButton::Primary),
            i.pointer.button_down(egui::PointerButton::Primary),
            i.pointer.press_origin(),
            i.pointer.latest_pos(),
        )
    });
    if pressed && resp.hovered() {
        ed.prepare_pointer_press();
        if ed.is_brush_tool() {
            if let Some(p) = origin.or(latest) {
                ed.begin(view.to_vss(p));
            }
        }
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        if !ed.is_dragging() {
            if let Some(p) = origin.or_else(|| resp.interact_pointer_pos()) {
                ed.begin(view.to_vss(p));
            }
        }
        if let Some(p) = resp.interact_pointer_pos() {
            ed.extend(view.to_vss(p));
        }
    } else if resp.dragged_by(egui::PointerButton::Primary) {
        if let Some(p) = resp.interact_pointer_pos() {
            ed.extend(view.to_vss(p));
        }
        ui.ctx().request_repaint();
    }
    if resp.drag_stopped_by(egui::PointerButton::Primary) || (released && ed.is_dragging()) {
        if let Some(p) = latest {
            ed.finish(view.to_vss(p));
        }
    }
    if resp.clicked_by(egui::PointerButton::Primary) {
        if let Some(p) = resp.interact_pointer_pos() {
            if ed.doc.tool == Tool::Select {
                ed.selected_item = ed
                    .doc
                    .topmost_hit(view.to_vss(p), 0.0)
                    .map(|(index, _)| index);
            }
            if ed.doc.tool == Tool::Text
                && ed.doc.selection.is_some_and(|s| s.contains(view.to_vss(p)))
            {
                ed.start_text(view.to_vss(p));
            }
            if ed.doc.tool == Tool::Sequence {
                ed.add_sequence(view.to_vss(p));
            }
        }
    }
    if resp.double_clicked_by(egui::PointerButton::Primary)
        && ed.doc.tool == Tool::Select
        && ed.doc.selection.is_some()
    {
        cmds.push(Command::CopyAndExit);
    }
    if resp.secondary_clicked() {
        if ed.text.is_some() {
            ed.commit_text();
        } else if ed.doc.selection.is_some() && !ed.is_snapping() {
            ed.reset_session(view.screen_rect(screen.size()), ed.window_snapping);
        } else {
            cmds.push(Command::Cancel);
        }
    }
    if ed.is_snapping() && !down {
        if let Some(hover) = resp.hover_pos() {
            let vss = view.to_vss(hover);
            if ed.take_snap_probe(vss) {
                let full = view.screen_rect(screen.size());
                match snipaste_platform::window_under_point(vss) {
                    Some(win) => ed.select_window(win, full),
                    None => ed.select_full_screen(full),
                }
                ui.ctx().request_repaint();
            }
        }
    }
    if resp.hovered() && !down {
        let mut cursor = egui::CursorIcon::Crosshair;
        if !ed.is_snapping() {
            if let (Some(sel), Some(hover)) = (ed.doc.selection, resp.hover_pos()) {
                let vss = view.to_vss(hover);
                if let Some(h) = sel.handle_at(vss, ed.handle_hit_tolerance()) {
                    cursor = handle_cursor(h);
                } else if ed.doc.tool == Tool::Select && sel.contains(vss) {
                    cursor = egui::CursorIcon::Move;
                } else if ed.doc.tool == Tool::Text {
                    cursor = egui::CursorIcon::Text;
                }
            }
        }
        ui.ctx().set_cursor_icon(cursor);
    }
    if resp.hovered() && ed.is_brush_tool() && !ed.is_snapping() {
        if let Some(p) = latest.filter(|p| {
            ed.doc
                .selection
                .is_some_and(|s| s.contains(view.to_vss(*p)))
        }) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("snipaste.brush.cursor"),
            ));
            let radius = view.len_to_points(ed.brush_width * 0.5);
            painter.circle_stroke(p, radius, Stroke::new(2.0, Color32::BLACK));
            painter.circle_stroke(p, radius, Stroke::new(1.0, Color32::WHITE));
        }
    }
}

fn handle_cursor(h: Handle) -> egui::CursorIcon {
    match h {
        Handle::LeftTop | Handle::RightBottom => egui::CursorIcon::ResizeNwSe,
        Handle::RightTop | Handle::LeftBottom => egui::CursorIcon::ResizeNeSw,
        Handle::Top | Handle::Bottom => egui::CursorIcon::ResizeVertical,
        Handle::Right | Handle::Left => egui::CursorIcon::ResizeHorizontal,
    }
}

const TOOLBAR_WIDTH: f32 = 598.0;
fn toolbar_position(screen: ERect, sel: ERect, height: f32) -> egui::Pos2 {
    let width = TOOLBAR_WIDTH.min(screen.width() - 8.0);
    let x = (sel.right() - width).clamp(
        screen.left() + 4.0,
        (screen.right() - width - 4.0).max(screen.left() + 4.0),
    );
    let y = if sel.bottom() + height + 10.0 <= screen.bottom() {
        sel.bottom() + 10.0
    } else if sel.top() - height - 10.0 >= screen.top() {
        sel.top() - height - 10.0
    } else {
        (sel.bottom() - height - 10.0).max(screen.top() + 4.0)
    };
    egui::pos2(x, y)
}

#[derive(Clone, Copy)]
enum Icon {
    Tool(Tool),
    Undo,
    Redo,
    Qr,
    Save,
    Pin,
    Cancel,
    Confirm,
    Clear,
}
fn toolbar(
    ctx: &Context,
    ed: &mut Editor,
    screen: ERect,
    sel: ERect,
    paste_hotkey: &str,
    cmds: &mut Vec<Command>,
) {
    let properties = ed.doc.tool != Tool::Select;
    let pos = toolbar_position(screen, sel, if properties { 78.0 } else { 40.0 });
    egui::Area::new(egui::Id::new("snipaste.toolbar"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 4.0);
            egui::Frame::NONE
                .fill(theme::TOOLBAR_BG)
                .corner_radius(3.0)
                .stroke(Stroke::new(1.0, Color32::from_gray(182)))
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for tool in Tool::ALL {
                            let hint =
                                format!("{}（{}）", tool.label(), tool.shortcut().unwrap_or(' '));
                            if icon_button(ui, Icon::Tool(tool), &hint, ed.doc.tool == tool, true)
                                .clicked()
                            {
                                ed.set_tool(if ed.doc.tool == tool {
                                    Tool::Select
                                } else {
                                    tool
                                });
                            }
                        }
                        separator(ui);
                        for (icon, hint, command, enabled) in [
                            (
                                Icon::Undo,
                                shortcut_hint("撤销（Ctrl+Z）", "撤销（⌘Z）"),
                                Command::Undo,
                                ed.can_undo(),
                            ),
                            (
                                Icon::Redo,
                                shortcut_hint("重做（Ctrl+Y）", "重做（⌘⇧Z / ⌘Y）"),
                                Command::Redo,
                                ed.can_redo(),
                            ),
                            (
                                Icon::Clear,
                                "清空标注",
                                Command::ClearAll,
                                ed.annotation_count() > 0,
                            ),
                        ] {
                            if icon_button(ui, icon, hint, false, enabled).clicked() {
                                cmds.push(command);
                            }
                        }
                        separator(ui);
                        let paste_hint = format!("贴到桌面（{paste_hotkey}）");
                        for (icon, hint, command) in [
                            (Icon::Qr, "识别选区内二维码（Q）", Command::RecognizeQr),
                            (Icon::Pin, paste_hint.as_str(), Command::Pin),
                            (
                                Icon::Save,
                                shortcut_hint("保存截图（Ctrl+S）", "保存截图（⌘S）"),
                                Command::SaveAs,
                            ),
                            (Icon::Cancel, "取消截图（Esc）", Command::Cancel),
                            (
                                Icon::Confirm,
                                shortcut_hint(
                                    "完成截图并复制（Enter / Ctrl+C）",
                                    "完成截图并复制（Enter / ⌘C）",
                                ),
                                Command::CopyAndExit,
                            ),
                        ] {
                            if icon_button(ui, icon, hint, false, true).clicked() {
                                cmds.push(command);
                            }
                        }
                    });
                    if properties {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if matches!(ed.doc.tool, Tool::Mosaic | Tool::Eraser) {
                                let brush = if ed.doc.tool == Tool::Mosaic {
                                    &mut ed.mosaic_brush
                                } else {
                                    &mut ed.eraser_brush
                                };
                                ui.selectable_value(brush, false, "框选");
                                ui.selectable_value(brush, true, "涂抹");
                                if *brush {
                                    ui.label("大小");
                                    ui.add(
                                        egui::Slider::new(&mut ed.brush_width, 4.0..=96.0)
                                            .suffix(" px"),
                                    );
                                }
                                if ed.doc.tool == Tool::Mosaic {
                                    ui.label("强度");
                                    ui.add(egui::Slider::new(&mut ed.style.mosaic_block, 4..=40));
                                }
                                return;
                            }
                            for color in PALETTE {
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(22.0, 24.0),
                                    egui::Sense::click(),
                                );
                                ui.painter().circle_filled(
                                    rect.center(),
                                    6.0,
                                    canvas::to_color32(color),
                                );
                                if ed.style.stroke == color {
                                    ui.painter().circle_stroke(
                                        rect.center(),
                                        9.0,
                                        Stroke::new(1.5, theme::ACCENT),
                                    );
                                }
                                if resp.clicked() {
                                    ed.style.stroke = color;
                                }
                            }
                            separator(ui);
                            match ed.doc.tool {
                                Tool::Sequence => {
                                    ui.label("大小");
                                    ui.add(
                                        egui::DragValue::new(&mut ed.sequence_size)
                                            .range(20.0..=96.0)
                                            .suffix(" px"),
                                    );
                                    ui.label(format!("下一个：{}", ed.next_sequence_number()));
                                }
                                Tool::Text => {
                                    ui.label(egui::RichText::new("字号").color(theme::TOOLBAR_FG));
                                    ui.add(
                                        egui::DragValue::new(&mut ed.style.font_size)
                                            .range(10.0..=96.0)
                                            .speed(1.0),
                                    );
                                }
                                _ => {
                                    for width in [2.0, 4.0, 8.0] {
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(26.0, 24.0),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().circle_filled(
                                            rect.center(),
                                            width * 0.6 + 1.0,
                                            theme::TOOLBAR_FG,
                                        );
                                        if (ed.style.stroke_width - width).abs() < 0.1 {
                                            ui.painter().rect_stroke(
                                                rect,
                                                2.0,
                                                Stroke::new(1.0, theme::ACCENT),
                                                StrokeKind::Inside,
                                            );
                                        }
                                        if resp.clicked() {
                                            ed.style.stroke_width = width;
                                        }
                                    }
                                    if matches!(ed.doc.tool, Tool::Rect | Tool::Ellipse) {
                                        let mut fill = ed.style.fill.is_some();
                                        if ui.checkbox(&mut fill, "填充").changed() {
                                            ed.style.fill = fill.then_some(ed.style.stroke);
                                        }
                                        if fill {
                                            ed.style.fill = Some(ed.style.stroke);
                                        }
                                    }
                                }
                            }
                        });
                    }
                });
        });
}

fn separator(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(6.0, 26.0), egui::Sense::hover());
    ui.painter().line_segment(
        [
            r.center_top() + egui::vec2(0.0, 4.0),
            r.center_bottom() - egui::vec2(0.0, 4.0),
        ],
        Stroke::new(1.0, Color32::from_gray(208)),
    );
}
fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    hint: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let (r, mut resp) = ui.allocate_exact_size(
        egui::vec2(30.0, 30.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if active || (enabled && resp.hovered()) {
        ui.painter().rect_filled(r, 2.0, theme::HOVER_BG);
        if active {
            ui.painter()
                .rect_stroke(r, 2.0, Stroke::new(1.0, theme::ACCENT), StrokeKind::Inside);
        }
    }
    let color = if !enabled {
        Color32::from_gray(190)
    } else if matches!(icon, Icon::Confirm) {
        Color32::from_rgb(26, 151, 82)
    } else if matches!(icon, Icon::Cancel) {
        Color32::from_rgb(202, 63, 63)
    } else if active {
        theme::ACCENT
    } else {
        theme::TOOLBAR_FG
    };
    paint_icon(ui.painter(), r.shrink(7.0), icon, color);
    resp = resp.on_hover_text(hint);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
fn paint_icon(p: &egui::Painter, r: ERect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.5, color);
    let point = |x: f32, y: f32| egui::pos2(r.left() + r.width() * x, r.top() + r.height() * y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([point(a.0, a.1), point(b.0, b.1)], s);
    };
    match icon {
        Icon::Tool(Tool::Select) => {
            line((0.1, 0.0), (0.1, 0.9));
            line((0.1, 0.0), (0.8, 0.6));
            line((0.1, 0.9), (0.4, 0.6));
            line((0.4, 0.6), (0.8, 0.6));
        }
        Icon::Tool(Tool::Rect) => {
            p.rect_stroke(r.shrink2(egui::vec2(0.0, 2.0)), 0.0, s, StrokeKind::Inside);
        }
        Icon::Tool(Tool::Ellipse) => {
            p.circle_stroke(r.center(), r.width() * 0.48, s);
        }
        Icon::Tool(Tool::Arrow) => {
            line((0.0, 1.0), (1.0, 0.0));
            line((0.4, 0.0), (1.0, 0.0));
            line((1.0, 0.0), (1.0, 0.6));
        }
        Icon::Tool(Tool::Pen) => {
            line((0.1, 0.9), (0.85, 0.15));
            line((0.25, 1.0), (1.0, 0.25));
            line((0.85, 0.15), (1.0, 0.25));
            line((0.1, 0.9), (0.25, 1.0));
        }
        Icon::Tool(Tool::Marker) => {
            line((0.15, 0.8), (0.7, 0.0));
            line((0.5, 1.0), (1.0, 0.3));
            line((0.7, 0.0), (1.0, 0.3));
            line((0.15, 0.8), (0.5, 1.0));
        }
        Icon::Tool(Tool::Mosaic) | Icon::Qr => {
            for y in 0..3 {
                for x in 0..3 {
                    if !matches!(icon, Icon::Qr) || x != 1 || y != 1 {
                        let a = point(x as f32 / 3.0, y as f32 / 3.0);
                        let cell = ERect::from_min_size(a, r.size() / 3.0 - egui::vec2(1.0, 1.0));
                        if (x + y) % 2 == 0 {
                            p.rect_filled(cell, 0.0, color);
                        } else {
                            p.rect_stroke(cell, 0.0, s, StrokeKind::Inside);
                        }
                    }
                }
            }
        }
        Icon::Tool(Tool::Text) => {
            line((0.0, 0.1), (1.0, 0.1));
            line((0.5, 0.1), (0.5, 1.0));
            line((0.25, 1.0), (0.75, 1.0));
        }
        Icon::Tool(Tool::Sequence) => {
            p.circle_stroke(r.center(), r.width() * 0.5, s);
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "1",
                FontId::proportional(r.height() * 0.85),
                color,
            );
        }
        Icon::Tool(Tool::Eraser) | Icon::Clear => {
            line((0.0, 0.7), (0.65, 0.0));
            line((0.65, 0.0), (1.0, 0.4));
            line((1.0, 0.4), (0.4, 1.0));
            line((0.4, 1.0), (0.0, 0.7));
            line((0.2, 0.5), (0.6, 0.9));
        }
        Icon::Undo | Icon::Redo => {
            let flip = matches!(icon, Icon::Redo);
            let q = |x: f32, y: f32| point(if flip { 1.0 - x } else { x }, y);
            p.add(egui::Shape::line(
                vec![
                    q(0.0, 0.4),
                    q(0.45, 0.25),
                    q(0.8, 0.35),
                    q(1.0, 0.65),
                    q(0.85, 0.95),
                ],
                s,
            ));
            p.line_segment([q(0.0, 0.4), q(0.25, 0.0)], s);
            p.line_segment([q(0.0, 0.4), q(0.4, 0.6)], s);
        }
        Icon::Save => {
            p.rect_stroke(r, 0.0, s, StrokeKind::Inside);
            line((0.2, 0.0), (0.2, 0.4));
            line((0.2, 0.4), (0.8, 0.4));
            line((0.8, 0.4), (0.8, 0.0));
            p.rect_stroke(
                ERect::from_min_max(point(0.2, 0.65), point(0.8, 1.0)),
                0.0,
                s,
                StrokeKind::Inside,
            );
        }
        Icon::Pin => {
            line((0.3, 0.0), (0.9, 0.3));
            line((0.4, 0.2), (0.25, 0.6));
            line((0.8, 0.4), (0.6, 0.75));
            line((0.25, 0.6), (0.6, 0.75));
            line((0.42, 0.7), (0.2, 1.0));
        }
        Icon::Cancel => {
            line((0.1, 0.1), (0.9, 0.9));
            line((0.9, 0.1), (0.1, 0.9));
        }
        Icon::Confirm => {
            p.line_segment([point(0.0, 0.5), point(0.35, 0.9)], Stroke::new(2.4, color));
            p.line_segment([point(0.35, 0.9), point(1.0, 0.1)], Stroke::new(2.4, color));
        }
    }
}

fn text_editor(ctx: &Context, ed: &mut Editor, view: Viewport, screen: ERect) {
    let Some(text) = ed.text.as_mut() else {
        return;
    };
    let pos = view.to_points(text.position);
    let width = (screen.right() - pos.x - 4.0).clamp(40.0, 360.0);
    let mut commit = false;
    egui::Area::new(egui::Id::new("snipaste.text"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_white_alpha(235))
                .stroke(Stroke::new(1.0, theme::ACCENT))
                .inner_margin(3)
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut text.content)
                            .desired_width(width)
                            .desired_rows(1)
                            .font(FontId::proportional(
                                view.len_to_points(text.style.font_size),
                            ))
                            .text_color(canvas::to_color32(text.style.stroke))
                            .frame(egui::Frame::NONE),
                    );
                    if text.focus {
                        response.request_focus();
                        text.focus = false;
                    }
                    let galley = ui.painter().layout_no_wrap(
                        text.content.clone(),
                        FontId::proportional(view.len_to_points(text.style.font_size)),
                        canvas::to_color32(text.style.stroke),
                    );
                    text.size = snipaste_core::Pos::new(
                        galley.size().x * view.scale,
                        galley.size().y * view.scale,
                    );
                    commit = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    if commit {
                        text.content = text.content.trim_end_matches('\n').to_owned();
                    }
                });
        });
    if commit {
        ed.commit_text();
    }
}

fn qr_result(ctx: &Context, ed: &mut Editor, cmds: &mut Vec<Command>) {
    let Some(result) = ed.qr.as_ref() else {
        return;
    };
    let mut open = true;
    egui::Window::new("二维码识别")
        .collapsible(false)
        .resizable(false)
        .default_width(390.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| match result {
            QrResult::Pending => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("正在识别选区…");
                });
                ctx.request_repaint();
            }
            QrResult::Found(items) => {
                ui.label(format!("识别到 {} 个二维码", items.len()));
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (i, text) in items.iter().enumerate() {
                            if items.len() > 1 {
                                ui.label(format!("二维码 {}", i + 1));
                            }
                            let mut content = text.clone();
                            ui.add(
                                egui::TextEdit::multiline(&mut content)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(2),
                            );
                        }
                    });
                if ui.button("复制识别结果").clicked() {
                    cmds.push(Command::CopyQr);
                }
            }
            QrResult::Error(message) => {
                ui.label(message);
                if ui.button("重新识别").clicked() {
                    cmds.push(Command::RecognizeQr);
                }
            }
        });
    if !open {
        ed.qr = None;
    }
}

fn magnifier(
    ctx: &Context,
    bg: &Background<'_>,
    view: Viewport,
    screen: ERect,
    format: ColorFormat,
) {
    let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let Some(sample) = color_picker::sample(bg.snapshot, view.to_vss(pointer)) else {
        return;
    };
    // 两种格式共用尺寸，切换时不会改变放大镜的位置或截断 RGB 三个通道。
    let size = egui::vec2(184.0, 238.0);
    let mut pos = pointer + egui::vec2(22.0, 22.0);
    if pos.x + size.x > screen.right() {
        pos.x = pointer.x - size.x - 22.0;
    }
    if pos.y + size.y > screen.bottom() {
        pos.y = pointer.y - size.y - 22.0;
    }
    pos.x = pos.x.max(screen.left());
    pos.y = pos.y.max(screen.top());
    egui::Area::new(egui::Id::new("snipaste.magnifier"))
        .interactable(false)
        .fixed_pos(pos)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            ui.set_width(170.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            egui::Frame::NONE
                .fill(Color32::from_gray(28))
                .stroke(Stroke::new(1.0, Color32::from_gray(180)))
                .inner_margin(6)
                .show(ui, |ui| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(170.0, 170.0), egui::Sense::hover());
                    ui.painter().image(
                        bg.texture.id(),
                        r,
                        color_picker::zoom_uv(bg.snapshot, sample.pixel),
                        Color32::WHITE,
                    );
                    ui.painter().rect_stroke(
                        ERect::from_center_size(r.center(), r.size() / 17.0),
                        0.0,
                        Stroke::new(1.0, theme::ACCENT),
                        StrokeKind::Inside,
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "({:.0}, {:.0})",
                            sample.position.x, sample.position.y
                        ))
                        .monospace()
                        .size(11.0)
                        .color(Color32::WHITE),
                    );
                    ui.horizontal(|ui| {
                        let (swatch, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(
                            swatch,
                            0.0,
                            Color32::from_rgb(sample.rgb[0], sample.rgb[1], sample.rgb[2]),
                        );
                        ui.painter().rect_stroke(
                            swatch,
                            0.0,
                            Stroke::new(1.0, Color32::from_gray(190)),
                            StrokeKind::Inside,
                        );
                        ui.label(
                            egui::RichText::new(format.label(sample.rgb))
                                .monospace()
                                .size(11.0)
                                .color(Color32::WHITE),
                        );
                    });
                    ui.label(
                        egui::RichText::new("Shift 切换 · C 复制")
                            .size(10.0)
                            .color(Color32::from_gray(190)),
                    );
                });
        });
}
#[cfg(test)]
mod tests {
    //! 用**无头 egui** 覆盖交互层。
    //!
    //! `handle_canvas_input` 的行为取决于 egui 自己的时序：拖拽要等指针离开
    //! 按下点超过 `InputOptions::max_click_dist`（0.36 里是 6 点）才被判为拖拽，
    //! 而那一刻 `interact_pointer_pos()` 早就不是按下点了。这种"时序型"缺陷
    //! 用 `Editor` 的单元测试完全碰不到 —— 线上反馈的"选区调不了大小"正是
    //! 这一层的问题，所以必须直接把合成事件喂给 `Context` 来测。

    use super::*;
    use snipaste_core::geometry::Pos;
    use snipaste_core::Rect;

    const SCREEN: f32 = 1000.0;

    /// 驱动一帧无头 egui。
    fn run_frame(
        ctx: &Context,
        ed: &mut Editor,
        events: Vec<egui::Event>,
        time: f64,
    ) -> Vec<Command> {
        let input = egui::RawInput {
            screen_rect: Some(ERect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SCREEN, SCREEN),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut cmds = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            cmds = draw(ui, ed, Viewport::new(Pos::ZERO, 1.0), None, "F1", "F3");
        });
        // egui 的 `TexturesDelta` 有 drop 断言（debug 下会 panic），
        // 无头测试里没有真实纹理要上传，显式 clear 掉。
        out.textures_delta.clear();
        cmds
    }

    fn button_event(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn select_editor() -> Editor {
        let mut ed = Editor::new();
        // 不设置工具：要覆盖的是程序启动时的真实默认（指针）。
        assert_eq!(ed.doc.tool, Tool::Select);
        ed.set_screen_bounds(Rect::new(0.0, 0.0, SCREEN, SCREEN));
        // 右下角在 (600, 500)
        ed.set_selection(Some(Rect::new(200.0, 200.0, 400.0, 300.0)));
        ed
    }

    #[test]
    fn clicking_with_sequence_tool_adds_exactly_one_number_per_click() {
        let ctx = Context::default();
        let mut ed = select_editor();
        ed.set_tool(Tool::Sequence);
        run_frame(&ctx, &mut ed, vec![], 0.0);
        for (i, x) in [300.0, 400.0, 500.0].into_iter().enumerate() {
            let p = egui::pos2(x, 350.0);
            let time = 1.0 + i as f64;
            run_frame(
                &ctx,
                &mut ed,
                vec![egui::Event::PointerMoved(p), button_event(p, true)],
                time,
            );
            run_frame(&ctx, &mut ed, vec![button_event(p, false)], time + 0.05);
            assert_eq!(ed.doc.items.len(), i + 1);
            assert!(
                matches!(ed.doc.items[i], snipaste_core::Annotation::Sequence { number, .. } if number == i as u32 + 1)
            );
        }
        assert_eq!(ed.history.undo_len(), 3);
    }

    /// 回归测试：角柄必须按**按下点**命中，而不是被 egui 判定为拖拽那一刻的
    /// 指针位置。
    ///
    /// 场景就是一帧之内从角柄甩到远处 —— 这在真实使用里很常见（鼠标快一点
    /// 就是一帧跨几十像素）。修复前 `begin` 收到的是 (700, 650)，那里既不在
    /// 选区内也不在任何角柄的命中半径里，于是被当成"在选区外拖出新选区"，
    /// 用户调好的选区直接被清掉。修复后按按下点 (600, 500) 命中右下角柄，
    /// 结果是正常的缩放。
    #[test]
    fn corner_handle_is_grabbed_from_the_press_point_not_the_latest_pointer() {
        let ctx = Context::default();
        let mut ed = select_editor();

        let corner = egui::pos2(600.0, 500.0);
        let far = egui::pos2(700.0, 650.0);

        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(corner)], 0.00);
        run_frame(&ctx, &mut ed, vec![button_event(corner, true)], 0.05);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(far)], 0.10);
        run_frame(&ctx, &mut ed, vec![button_event(far, false)], 0.15);

        let sel = ed.doc.selection.expect("选区不该消失");
        assert_eq!(sel.min, Pos::new(200.0, 200.0), "左上角不该动");
        assert_eq!(
            sel.max,
            Pos::new(700.0, 650.0),
            "右下角应被拖到 (700, 650)；若这里得到 (600, 500) 说明按下点被当成了拖拽起点"
        );
        assert_eq!(ed.history.undo_len(), 0, "调选区不是文档编辑");
    }

    /// 回归测试：指针一按下就必须退出窗口吸附。
    ///
    /// egui 判定"这是拖拽"要等指针移动超过 6 点，而吸附探针的去抖步长恰好
    /// 也是 6 —— 若等到 `drag_started()` 才退出吸附，中间那几帧里吸附会把
    /// 用户正要拖的选区整个换成指针下窗口的矩形。
    #[test]
    fn pointer_down_disarms_window_snapping_immediately() {
        let ctx = Context::default();
        let mut ed = select_editor();
        ed.arm_snap();

        let p = egui::pos2(400.0, 350.0);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(p)], 0.00);
        assert!(ed.is_snapping(), "还没动手时吸附应当仍开着");

        run_frame(&ctx, &mut ed, vec![button_event(p, true)], 0.05);
        assert!(
            !ed.is_snapping(),
            "按下的一瞬间就该退出吸附，而不是等 egui 把它判成拖拽"
        );
    }

    /// 选区内按下（远离任何角柄）应当是整体平移，不能被角柄命中区抢走。
    #[test]
    fn pressing_well_inside_the_selection_moves_it() {
        let ctx = Context::default();
        let mut ed = select_editor();

        let start = egui::pos2(400.0, 350.0);
        let end = egui::pos2(450.0, 400.0);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(start)], 0.00);
        run_frame(&ctx, &mut ed, vec![button_event(start, true)], 0.05);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(end)], 0.10);
        run_frame(&ctx, &mut ed, vec![button_event(end, false)], 0.15);

        let sel = ed.doc.selection.expect("选区不该消失");
        assert_eq!(sel.width(), 400.0, "整体平移不应改变尺寸");
        assert_eq!(sel.height(), 300.0);
        assert_eq!(sel.min, Pos::new(250.0, 250.0));
    }

    #[test]
    fn dragging_inside_preview_freely_selects_instead_of_moving_window() {
        let ctx = Context::default();
        let mut ed = select_editor();
        ed.arm_snap();
        let start = egui::pos2(400.0, 350.0);
        let end = egui::pos2(490.0, 410.0);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(start)], 0.0);
        run_frame(
            &ctx,
            &mut ed,
            vec![egui::Event::PointerMoved(start), button_event(start, true)],
            0.05,
        );
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(end)], 0.1);
        run_frame(&ctx, &mut ed, vec![button_event(end, false)], 0.2);
        assert_eq!(ed.doc.selection, Some(Rect::new(400.0, 350.0, 90.0, 60.0)));
    }

    #[test]
    fn confirm_and_qr_buttons_emit_actions_without_changing_selection_or_tool() {
        for (offset, expected) in [(15.0, Command::CopyAndExit), (143.0, Command::RecognizeQr)] {
            let ctx = Context::default();
            let mut ed = select_editor();
            let before = ed.doc.selection;
            // 工具条与选区右边对齐；最后一个按钮为勾号，左移四格是二维码。
            let p = egui::pos2(600.0 - 4.0 - offset, 529.0);
            run_frame(&ctx, &mut ed, vec![], 0.0);
            run_frame(&ctx, &mut ed, vec![], 0.01);
            run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(p)], 0.02);
            run_frame(&ctx, &mut ed, vec![button_event(p, true)], 0.03);
            let commands = run_frame(&ctx, &mut ed, vec![button_event(p, false)], 0.04);
            assert!(
                commands.contains(&expected),
                "点击应产生 {expected:?}，实际 {commands:?}"
            );
            assert_eq!(ed.doc.selection, before);
            assert_eq!(ed.doc.tool, Tool::Select);
            assert!(ed.doc.items.is_empty());
        }
    }

    #[test]
    fn toolbar_fits_bottom_and_right_screen_edges() {
        let screen = ERect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        let selection = ERect::from_min_max(egui::pos2(920.0, 730.0), egui::pos2(999.0, 799.0));
        let pos = toolbar_position(screen, selection, 78.0);
        assert!(pos.x + TOOLBAR_WIDTH <= screen.right());
        assert!(pos.y + 78.0 < selection.top());
    }

    #[test]
    fn mosaic_toolbar_then_reverse_drag_commits_a_real_region() {
        let ctx = Context::default();
        let mut ed = select_editor();
        // 第七个图标为马赛克，通过真实按钮切换，覆盖按钮命中与画布输入链路。
        let button = egui::pos2(600.0 - TOOLBAR_WIDTH + 4.0 + 6.0 * 32.0 + 15.0, 529.0);
        run_frame(&ctx, &mut ed, vec![], 0.0);
        run_frame(&ctx, &mut ed, vec![], 0.01);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(button)], 0.02);
        run_frame(&ctx, &mut ed, vec![button_event(button, true)], 0.03);
        run_frame(&ctx, &mut ed, vec![button_event(button, false)], 0.04);
        assert_eq!(ed.doc.tool, Tool::Mosaic);
        let start = egui::pos2(480.0, 400.0);
        let end = egui::pos2(290.0, 270.0);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(start)], 0.1);
        run_frame(&ctx, &mut ed, vec![button_event(start, true)], 0.2);
        run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(end)], 0.3);
        run_frame(&ctx, &mut ed, vec![button_event(end, false)], 0.4);
        assert!(
            matches!(&ed.doc.items[0], snipaste_core::Annotation::Mosaic {
            region: snipaste_core::annotation::MosaicRegion::Rect(r), ..
        } if *r == Rect::new(290.0, 270.0, 190.0, 130.0))
        );
        assert_eq!(
            ed.doc.selection,
            Some(Rect::new(200.0, 200.0, 400.0, 300.0))
        );
    }

    #[test]
    fn brush_click_and_fast_release_each_make_one_undoable_stroke() {
        for tool in [Tool::Eraser, Tool::Mosaic] {
            let ctx = Context::default();
            let mut ed = select_editor();
            ed.mosaic_brush = true;
            ed.set_tool(tool);
            let start = egui::pos2(280.0, 350.0);
            let end = egui::pos2(550.0, 350.0);
            run_frame(&ctx, &mut ed, vec![egui::Event::PointerMoved(start)], 0.0);
            run_frame(&ctx, &mut ed, vec![button_event(start, true)], 0.1);
            assert!(ed.doc.draft.is_some(), "按下立即显示笔刷");
            run_frame(&ctx, &mut ed, vec![button_event(start, false)], 0.2);
            assert_eq!(ed.doc.items.len(), 1, "单击也要生效");
            run_frame(&ctx, &mut ed, vec![button_event(start, true)], 0.4);
            run_frame(
                &ctx,
                &mut ed,
                vec![egui::Event::PointerMoved(end), button_event(end, false)],
                0.5,
            );
            assert_eq!(ed.history.undo_len(), 2);
            assert!(
                ed.doc.items[1].hit_test(Pos::new(400.0, 350.0), 0.0),
                "稀疏事件间的线段要连续覆盖"
            );
            assert!(ed.undo());
            assert_eq!(ed.doc.items.len(), 1);
            assert!(ed.redo());
            assert_eq!(ed.doc.items.len(), 2);
        }
    }

    #[test]
    fn text_bounds_match_displayed_content_for_selection_and_erasing() {
        let ctx = Context::default();
        let mut ed = select_editor();
        ed.set_tool(Tool::Text);
        ed.start_text(Pos::new(250.0, 300.0));
        ed.text.as_mut().unwrap().content = "Example screenshot annotation".into();
        run_frame(&ctx, &mut ed, vec![], 0.0);
        ed.commit_text();
        let annotation = &ed.doc.items[0];
        let bounds = annotation.bounds();
        assert!(bounds.width() > 100.0);
        assert!(annotation.hit_test(Pos::new(bounds.center().x, 310.0), 0.0));
    }

    #[test]
    fn selected_area_alt_magnifier_shows_both_formats_without_changing_selection() {
        let ctx = Context::default();
        let mut ed = select_editor();
        let selection = ed.doc.selection;
        let snapshot = snipaste_capture::Snapshot {
            origin: Pos::ZERO,
            image: image::RgbaImage::from_pixel(1000, 1000, image::Rgba([18, 52, 86, 255])),
        };
        let texture = ctx.load_texture(
            "picker.test",
            egui::ColorImage::new(
                [1000, 1000],
                vec![Color32::from_rgb(18, 52, 86); 1000 * 1000],
            ),
            egui::TextureOptions::NEAREST,
        );
        let render = |ed: &mut Editor, alt: bool| {
            let mut full = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(ERect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(SCREEN, SCREEN),
                    )),
                    events: vec![
                        egui::Event::ModifiersChanged(egui::Modifiers {
                            alt,
                            ..Default::default()
                        }),
                        egui::Event::PointerMoved(egui::pos2(400.4, 350.7)),
                    ],
                    ..Default::default()
                },
                |ui| {
                    draw(
                        ui,
                        ed,
                        Viewport::new(Pos::ZERO, 1.0),
                        Some(Background {
                            snapshot: &snapshot,
                            texture: &texture,
                        }),
                        "F1",
                        "F3",
                    );
                },
            );
            let labels = full
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            full.textures_delta.clear();
            labels
        };
        render(&mut ed, true);
        assert!(render(&mut ed, true)
            .iter()
            .any(|text| text == "HEX: #123456"));
        ed.color_format.toggle();
        let labels = render(&mut ed, true);
        assert!(labels.iter().any(|text| text == "RGB: (18, 52, 86)"));
        assert!(
            labels.iter().any(|text| text == "(400, 350)"),
            "坐标要与实际采样的像素一致"
        );
        assert!(!render(&mut ed, false)
            .iter()
            .any(|text| text.starts_with("RGB:")));
        assert_eq!(ed.doc.selection, selection);
        assert!(ed.doc.items.is_empty());
        ed.reset_session(snapshot.bounds(), true);
        assert_eq!(ed.color_format, ColorFormat::Rgb, "新截图保留格式偏好");
    }
}
