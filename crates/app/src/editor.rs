//! 编辑状态机：把"按下 / 移动 / 抬起"翻译成 `Document` 上的编辑操作。
//!
//! ## 为什么从事件循环里拆出来
//!
//! 这段逻辑是整个应用里最容易出错、也最值得测试的部分（草稿生命周期、
//! 采样抽稀、撤销栈登记时机）。如果它长在 winit 的 `window_event` 里，
//! 就只能靠开窗口手动点来验证。拆出来之后，用 `Pos` 序列就能完整覆盖。
//!
//! 所有传入的坐标都是 **VSS 物理像素**（调用方负责用 `Viewport` 换算）。

use crate::color_picker::ColorFormat;
use snipaste_core::annotation::MosaicRegion;
use snipaste_core::geometry::{Handle, Pos, Rect, HANDLE_HIT_RADIUS};
use snipaste_core::simplify::{accept_sample, douglas_peucker};
use snipaste_core::{Annotation, ArrowKind, Document, Edit, History, PathKind, Style, Tool};

/// 自由笔迹的抽稀容差（物理像素）。
///
/// 1.2 px 是经验值：再小则抽稀几乎无效，再大则手写的顿笔会被抹平成折线。
const SIMPLIFY_EPSILON: f32 = 1.2;

/// 采样点的最小间距（物理像素）。
///
/// 高回报率鼠标一秒能产生上千个点。1.5 px 的间距对人手来说完全无损，
/// 但能把点数压掉一个数量级 —— 这是"撤销栈无上限保留"能成立的前提之一。
const MIN_SAMPLE_DISTANCE: f32 = 1.5;

/// 选区允许的最小边长（物理像素）。
///
/// 小于这个尺寸的选区没有意义（导出会得到一条线），所以缩放时钳住，
/// 新建选区时低于这个尺寸则视为"误点"、保持原选区不动。
pub const MIN_SELECTION_SIZE: f32 = 8.0;

// 角柄的命中半径**不在这里定义** —— 用 `snipaste_core::HANDLE_HIT_RADIUS`。
//
// 角柄的边长（`HANDLE_SIZE`）由渲染层用来画、命中层用来判，两者必须是
// 同一个常量。历史上这里和 `render/canvas.rs` 各写了一份（8 点 / 6px 外扩），
// 结果"画出来的角柄"和"点得中的区域"对不上，表现就是角柄抓不住。

/// 吸附模式下，指针移动超过这个距离才重新查询一次窗口。
///
/// `WindowFromPoint` 之类的调用虽然不慢，但每帧都查会白白吃掉 CPU，
/// 而且窗口边界附近的抖动会让选区闪。物理像素。
const SNAP_PROBE_STEP: f32 = 6.0;

/// 一次拖拽的语义。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// 没有拖拽进行中（悬停或已结束）。
    Idle,
    /// 拖出一个几何图元（矩形 / 椭圆 / 箭头 / 马赛克框）。
    Shape,
    /// 自由绘制（画笔 / 记号笔）。
    Freehand,
    /// 整体平移选区。
    MoveSelection,
    MoveAnnotation {
        index: usize,
        start: Pos,
    },
    /// 拖角柄缩放选区。
    ///
    /// `grab_offset` 是"按下时角柄位置 − 按下点"。保留这个偏移，
    /// 角柄才不会在按下的瞬间跳到指针正中心（手抖一点就会有的观感问题）。
    ResizeSelection {
        handle: Handle,
        grab_offset: Pos,
    },
    /// 从选区外拖出一个新选区。
    ///
    /// `previous` 用于"其实只是点了一下"的兜底：松手时若新选区小得没意义，
    /// 就还原成 `previous`，而不是粗暴地把用户原来的选区清掉。
    NewSelection {
        anchor: Pos,
        previous: Option<Rect>,
    },
}

/// 应用层的可编辑状态。
pub struct Editor {
    pub doc: Document,
    pub history: History,
    /// 待用样式。`Document::style` 是文档里已提交图元的样式基线，
    /// 这里是"下一次绘制会用的"，两者刻意分开。
    pub style: Style,
    /// 最近一条给用户看的一句话提示（画在状态栏）。
    ///
    /// 目前用于反馈"这个功能还没实现"，避免用户点了按钮却毫无动静、
    /// 以为是程序卡死。
    pub hint: Option<String>,
    pub text: Option<TextDraft>,
    pub qr: Option<QrResult>,
    pub selected_item: Option<usize>,
    pub constrain: bool,
    pub mosaic_brush: bool,
    pub eraser_brush: bool,
    pub brush_width: f32,
    pub color_format: ColorFormat,
    pub sequence_size: f32,
    pub window_snapping: bool,
    anchor: Pos,
    capture_drag: bool,
    drag: Drag,
    /// 上一次采样点（VSS）。
    last: Pos,

    /// 可选区的边界（VSS 物理像素），通常是虚拟桌面并集。
    ///
    /// 拖拽时用它钳制选区，避免把选区拖到屏幕外面去 —— 那样用户既看不见、
    /// 也拖不回来。`None` 表示不限制（单元测试里就让它放开）。
    screen_bounds: Option<Rect>,

    /// 是否处于"窗口吸附"模式：指针移到哪个窗口上，选区就贴到那个窗口。
    ///
    /// 呼出遮罩时打开（对应 Snipaste 的默认行为），用户一旦自己动鼠标
    /// 拖拽/缩放就关掉 —— 否则会发生"我刚调好的选区被吸附改回去了"。
    snap_mode: bool,
    /// 上一次吸附探测时的指针位置，用于按 [`SNAP_PROBE_STEP`] 去抖。
    snap_probe: Option<Pos>,
}

pub struct TextDraft {
    pub position: Pos,
    pub content: String,
    pub style: Style,
    pub focus: bool,
    pub size: Pos,
}

pub enum QrResult {
    Pending,
    Found(Vec<String>),
    Error(String),
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
            history: History::new(History::DEFAULT_CAPACITY),
            style: Style::default(),
            hint: None,
            text: None,
            qr: None,
            selected_item: None,
            constrain: false,
            mosaic_brush: false,
            eraser_brush: true,
            brush_width: 24.0,
            color_format: ColorFormat::default(),
            sequence_size: 32.0,
            window_snapping: true,
            anchor: Pos::ZERO,
            capture_drag: false,
            drag: Drag::Idle,
            last: Pos::ZERO,
            screen_bounds: None,
            snap_mode: false,
            snap_probe: None,
        }
    }

    /// 记一条一次性提示。
    pub fn note(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::info!("{msg}");
        self.hint = Some(msg);
    }

    /// 设定可选区边界（VSS 物理像素）。由 `app` 层按显示器并集传入。
    pub fn set_screen_bounds(&mut self, bounds: Rect) {
        self.screen_bounds = Some(bounds);
    }

    /// 把点钳制到可选区边界内。
    fn clamp_point(&self, p: Pos) -> Pos {
        match self.screen_bounds {
            Some(b) if !b.is_empty() => {
                Pos::new(p.x.clamp(b.min.x, b.max.x), p.y.clamp(b.min.y, b.max.y))
            }
            _ => p,
        }
    }

    /// 把矩形整体挪进可选区边界内（不会改变尺寸）。
    fn clamp_rect(&self, r: Rect) -> Rect {
        match self.screen_bounds {
            Some(b) if !b.is_empty() => r.clamp_within(&b),
            _ => r,
        }
    }

    // ------------------------------------------------------------ 工具与选区

    pub fn set_tool(&mut self, tool: Tool) {
        self.commit_text();
        self.cancel_drag();
        // `Document::set_tool` 会顺带结束草稿与文字编辑状态
        self.doc.set_tool(tool);
        self.drag = Drag::Idle;
    }

    pub fn reset_session(&mut self, screen: Rect, snapping: bool) {
        let style = self.style;
        let brushes = (self.mosaic_brush, self.eraser_brush, self.brush_width);
        let color_format = self.color_format;
        let sequence_size = self.sequence_size;
        let window_snapping = self.window_snapping;
        *self = Self::new();
        self.style = style;
        (self.mosaic_brush, self.eraser_brush, self.brush_width) = brushes;
        self.color_format = color_format;
        self.sequence_size = sequence_size;
        self.window_snapping = window_snapping;
        self.set_screen_bounds(screen);
        if snapping {
            self.select_full_screen(screen);
            self.arm_snap();
        }
    }

    /// 预选窗口中的拖动始终自由框选；单击则锁定预选窗口。
    pub fn prepare_pointer_press(&mut self) {
        self.capture_drag = self.is_snapping();
        self.disarm_snap();
    }

    pub fn start_text(&mut self, p: Pos) {
        self.commit_text();
        self.text = Some(TextDraft {
            position: p,
            content: String::new(),
            style: self.style,
            focus: true,
            size: Pos::ZERO,
        });
    }

    pub fn commit_text(&mut self) {
        let Some(text) = self.text.take() else {
            return;
        };
        if text.content.trim().is_empty() {
            return;
        }
        self.doc.draft = Some(Annotation::Text {
            rect: Rect::new(
                text.position.x,
                text.position.y,
                text.size.x.max(text.style.font_size),
                text.size.y.max(text.style.font_size),
            ),
            content: text.content,
            style: text.style,
        });
        if let Some(edit) = self.doc.commit_draft() {
            self.history.record(edit);
        }
    }

    pub fn nudge_selection(&mut self, dx: f32, dy: f32, resize: bool) {
        self.disarm_snap();
        if let Some(sel) = self.doc.selection {
            self.doc.selection = Some(if resize {
                sel.resize(
                    Handle::RightBottom,
                    self.clamp_point(sel.max + Pos::new(dx, dy)),
                    MIN_SELECTION_SIZE,
                )
            } else {
                self.clamp_rect(sel.translate(dx, dy))
            });
        }
    }

    /// 设定选区。`Rect` 会自动规范化（保证 min < max）。
    #[allow(dead_code)] // 多显示器版本里由各屏的 Viewport 初始化，当前只有测试在用
    pub fn set_selection(&mut self, r: Option<Rect>) {
        self.doc.selection = r;
    }

    /// 选区 = 整块屏幕。这是"没找到可吸附的窗口"时的兜底。
    pub fn select_full_screen(&mut self, screen: Rect) {
        if screen.is_empty() {
            return;
        }
        self.doc.selection = Some(screen);
        log::debug!("选区 = 整屏 {screen:?}");
    }

    /// 选区 = 某个窗口的矩形（吸附命中时走这条）。
    ///
    /// 会先把窗口矩形与屏幕求交：窗口有一半在屏幕外的情形很常见
    /// （最大化后残留的透明边框、跨屏窗口），直接拿来当选区会让用户
    /// 看到一个超出屏幕的框。
    pub fn select_window(&mut self, window: Rect, screen: Rect) {
        let fitted = window.intersect(&screen).filter(|r| !r.is_empty());
        if let Some(r) = fitted {
            self.doc.selection = Some(r);
            log::debug!("选区 = 窗口 {r:?}（原始 {window:?}）");
        }
    }

    // ------------------------------------------------------------ 窗口吸附

    /// 打开吸附模式：指针移到哪个窗口上，选区就贴到哪个窗口。
    pub fn arm_snap(&mut self) {
        self.snap_mode = true;
        self.snap_probe = None;
    }

    /// 关闭吸附模式。用户一旦自己动过手，就不该再自动改他的选区。
    pub fn disarm_snap(&mut self) {
        self.snap_mode = false;
        self.snap_probe = None;
    }

    pub fn is_snapping(&self) -> bool {
        self.snap_mode
    }

    /// 吸附模式下是否需要重新探测一次窗口。
    ///
    /// 返回 `true` 时**会顺带记下当前位置**，因此调用方直接在返回 `true`
    /// 的分支里查询窗口即可，不必自己维护去抖状态。
    pub fn take_snap_probe(&mut self, pointer: Pos) -> bool {
        if !self.snap_mode {
            return false;
        }
        match self.snap_probe {
            Some(prev) if prev.distance(pointer) < SNAP_PROBE_STEP => false,
            _ => {
                self.snap_probe = Some(pointer);
                true
            }
        }
    }

    // ------------------------------------------------------------ 拖拽生命周期

    /// 角柄命中所需的半径（物理像素）。
    ///
    /// UI 层要用它做悬停反馈（换成缩放光标），`begin` 要用它做实际命中 ——
    /// 走同一个函数，避免"光标变了但拖不动"这种对不上。
    pub fn handle_hit_tolerance(&self) -> f32 {
        HANDLE_HIT_RADIUS
    }

    /// 当前选区是否就是"整块屏幕"。
    ///
    /// 整屏选区不是一个真的选区 —— 它只是"用户还没选"的占位（呼出遮罩后、
    /// 指针停在桌面上时就会出现）。在它内部按下必须是**重新框选**，
    /// 否则用户会得到"点了没反应"：平移一个铺满屏幕的矩形，屏幕上什么都不会变。
    fn selection_is_full_screen(&self) -> bool {
        match (self.doc.selection, self.screen_bounds) {
            (Some(sel), Some(bounds)) => !bounds.is_empty() && sel == bounds,
            _ => false,
        }
    }

    /// 指针工具按下时的分派：抓角柄缩放 / 选区内平移 / 选区外新建。
    ///
    /// 优先级不能调换：角柄压在选区边界上，如果先判 `contains`，
    /// 边上的角柄就永远抓不到，用户会发现"选区只能整体挪、改不了大小"。
    fn begin_selection_drag(&mut self, p: Pos) {
        let tolerance = self.handle_hit_tolerance();
        match self.doc.selection {
            Some(sel) => {
                if let Some(handle) = sel.handle_at(p, tolerance) {
                    let hp = handle.position_on(&sel);
                    self.drag = Drag::ResizeSelection {
                        handle,
                        // 保留"角柄与按下点"的相对位置，按下瞬间不跳
                        grab_offset: hp - p,
                    };
                } else if sel.contains(p) && !self.selection_is_full_screen() {
                    if let Some((index, _)) = self.doc.topmost_hit(p, 0.0) {
                        self.selected_item = Some(index);
                        self.drag = Drag::MoveAnnotation { index, start: p };
                    } else {
                        self.selected_item = None;
                        self.drag = Drag::MoveSelection;
                    }
                } else {
                    // 选区外按下（或选区还只是整屏占位）= 重新框一个。
                    // 这也是"任意调整"的一部分：按住就能从头框出想要的区域。
                    self.drag = Drag::NewSelection {
                        anchor: p,
                        previous: Some(sel),
                    };
                }
            }
            None => {
                self.drag = Drag::NewSelection {
                    anchor: p,
                    previous: None,
                };
            }
        }
    }

    /// 鼠标按下。
    pub fn begin(&mut self, p: Pos) {
        // 用户开始动手了，退出吸附模式 —— 否则刚调好的选区会被下一帧的
        // 吸附结果覆盖回去，看起来像"我拖不动"。
        let capture_drag = self.capture_drag || self.is_snapping();
        self.capture_drag = false;
        self.disarm_snap();

        let p = self.clamp_point(p);
        self.last = p;
        self.anchor = p;
        self.drag = Drag::Idle;

        if capture_drag || (self.doc.selection.is_none() && self.screen_bounds.is_some()) {
            self.drag = Drag::NewSelection {
                anchor: p,
                previous: self.doc.selection,
            };
            return;
        }
        if self.doc.tool != Tool::Select {
            if let Some(sel) = self.doc.selection {
                if sel.handle_at(p, self.handle_hit_tolerance()).is_some() || !sel.contains(p) {
                    self.begin_selection_drag(p);
                    return;
                }
            }
        }
        match self.doc.tool {
            Tool::Select => self.begin_selection_drag(p),
            Tool::Rect => {
                self.doc.draft = Some(Annotation::Rect {
                    rect: Rect::from_points(p, p),
                    style: self.style,
                });
                self.drag = Drag::Shape;
            }
            Tool::Ellipse => {
                self.doc.draft = Some(Annotation::Ellipse {
                    rect: Rect::from_points(p, p),
                    style: self.style,
                });
                self.drag = Drag::Shape;
            }
            Tool::Arrow => {
                self.doc.draft = Some(Annotation::Arrow {
                    from: p,
                    to: p,
                    kind: ArrowKind::Straight,
                    style: self.style,
                });
                self.drag = Drag::Shape;
            }
            Tool::Pen => {
                self.doc.draft = Some(Annotation::Path {
                    points: vec![p],
                    kind: PathKind::Pen,
                    style: self.style,
                });
                self.drag = Drag::Freehand;
            }
            Tool::Marker => {
                self.doc.draft = Some(Annotation::Path {
                    points: vec![p],
                    kind: PathKind::Marker,
                    // 记号笔用半透明色，模拟马克笔叠色
                    style: Style::for_marker(self.style.stroke, self.style.stroke_width),
                });
                self.drag = Drag::Freehand;
            }
            Tool::Mosaic | Tool::Eraser => {
                let brush = self.is_brush_tool();
                let region = if brush {
                    MosaicRegion::Path(vec![p])
                } else {
                    MosaicRegion::Rect(Rect::from_points(p, p))
                };
                let style = Style {
                    stroke_width: self.brush_width,
                    ..self.style
                };
                self.doc.draft = Some(if self.doc.tool == Tool::Mosaic {
                    Annotation::Mosaic { region, style }
                } else {
                    Annotation::Eraser { region, style }
                });
                self.selected_item = None;
                self.drag = if brush { Drag::Freehand } else { Drag::Shape };
            }
            Tool::Text => {
                self.start_text(p);
            }
            Tool::Sequence => {}
        }
    }

    /// Derive the next number from the document so undo/redo and new sessions
    /// cannot leave a separate counter out of sync.
    pub fn next_sequence_number(&self) -> u32 {
        self.doc
            .items
            .iter()
            .filter_map(|item| match item {
                Annotation::Sequence { number, .. } => Some(*number),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn add_sequence(&mut self, center: Pos) {
        if !self.doc.selection.is_some_and(|s| s.contains(center)) {
            return;
        }
        self.commit_text();
        self.cancel_drag();
        self.disarm_snap();
        let number = self.next_sequence_number();
        self.doc.draft = Some(Annotation::Sequence {
            center,
            number,
            radius: self.sequence_size.clamp(20.0, 96.0) * 0.5,
            style: self.style,
        });
        if let Some(edit) = self.doc.commit_draft() {
            self.history.record(edit);
        }
        self.selected_item = self.doc.items.len().checked_sub(1);
    }

    /// 鼠标移动（按住时）。
    pub fn extend(&mut self, p: Pos) {
        let p = self.constrained_endpoint(p);
        match self.drag {
            Drag::Shape => {
                self.update_shape(p);
            }
            Drag::Freehand => {
                let Some(points) = self.draft_points_mut() else {
                    return;
                };
                if !accept_sample(points, p, MIN_SAMPLE_DISTANCE) {
                    return;
                }
                points.push(p);
                self.last = p;
            }
            Drag::MoveSelection => {
                let dx = p.x - self.last.x;
                let dy = p.y - self.last.y;
                if let Some(sel) = self.doc.selection {
                    self.doc.selection = Some(self.clamp_rect(sel.translate(dx, dy)));
                }
                self.last = p;
            }
            Drag::MoveAnnotation { index, .. } => {
                if let Some(item) = self.doc.items.get_mut(index) {
                    item.translate(p.x - self.last.x, p.y - self.last.y);
                }
                self.last = p;
            }
            Drag::ResizeSelection {
                handle,
                grab_offset,
            } => {
                if let Some(sel) = self.doc.selection {
                    // 钳制的是"目标点"而不是结果矩形：后者会让选区在碰到
                    // 屏幕边缘时整体平移，出现"拖右边缘、左边缘也跟着跑"。
                    let target = self.clamp_point(p + grab_offset);
                    self.doc.selection = Some(sel.resize(handle, target, MIN_SELECTION_SIZE));
                }
            }
            Drag::NewSelection { anchor, .. } => {
                let p = self.clamp_point(p);
                self.doc.selection = Some(Rect::from_points(anchor, p));
            }
            Drag::Idle => {}
        }
    }

    /// 鼠标抬起。
    pub fn finish(&mut self, p: Pos) {
        let p = self.constrained_endpoint(p);
        match self.drag {
            Drag::Freehand => {
                if let Some(points) = self.draft_points_mut() {
                    if points.last().is_none_or(|l| l.distance(p) >= 0.5) {
                        points.push(p);
                    }
                    // 抽稀放在**最后**做：拖动过程中保持原始采样点，笔迹跟手；
                    // 松手时才压缩，用户看不到压缩过程。
                    let simplified = douglas_peucker(points, SIMPLIFY_EPSILON);
                    *points = simplified;
                }
            }
            Drag::Shape => {
                self.update_shape(p);
            }
            Drag::ResizeSelection {
                handle,
                grab_offset,
            } => {
                // 按下即抬起（没经过 extend）时也要落位，否则最后一点点位移会丢
                if let Some(sel) = self.doc.selection {
                    let target = self.clamp_point(p + grab_offset);
                    self.doc.selection = Some(sel.resize(handle, target, MIN_SELECTION_SIZE));
                }
            }
            Drag::NewSelection { anchor, previous } => {
                let p = self.clamp_point(p);
                let next = Rect::from_points(anchor, p);
                // 只是点了一下、没真的拖出面积 → 当作误触，还原原选区，
                // 而不是把用户辛苦调好的框清掉
                let usable =
                    next.width() >= MIN_SELECTION_SIZE && next.height() >= MIN_SELECTION_SIZE;
                self.doc.selection = if usable { Some(next) } else { previous };
            }
            Drag::MoveSelection => {
                // 补上"最后一次 `extend` 到松手"之间的位移。
                // 别的分支（Shape / Freehand / ResizeSelection）都会在
                // `finish` 里落位，这里如果留空，"按下—移动—松手"发生在同一帧
                // 时整段位移会被丢掉，表现为"拖了没反应"。
                if let Some(sel) = self.doc.selection {
                    let dx = p.x - self.last.x;
                    let dy = p.y - self.last.y;
                    if dx != 0.0 || dy != 0.0 {
                        self.doc.selection = Some(self.clamp_rect(sel.translate(dx, dy)));
                    }
                }
            }
            Drag::MoveAnnotation { index, start } => {
                if let Some(item) = self.doc.items.get_mut(index) {
                    item.translate(p.x - self.last.x, p.y - self.last.y);
                }
                let dx = p.x - start.x;
                let dy = p.y - start.y;
                if dx != 0.0 || dy != 0.0 {
                    self.history.record(Edit::Translate {
                        indices: vec![index],
                        dx,
                        dy,
                    });
                }
            }
            Drag::Idle => {}
        }

        self.drag = Drag::Idle;
        if let Some(edit) = self.doc.commit_draft() {
            self.history.record(edit);
        }
        // 拖拽结束时的选区快照。端到端脚本靠这一行判断"拖角柄到底有没有生效"，
        // 比在截图上做像素推断可靠得多。
        log::debug!("拖拽结束：选区 = {:?}", self.doc.selection);
    }

    pub fn is_brush_tool(&self) -> bool {
        (self.doc.tool == Tool::Mosaic && self.mosaic_brush)
            || (self.doc.tool == Tool::Eraser && self.eraser_brush)
    }

    fn draft_points_mut(&mut self) -> Option<&mut Vec<Pos>> {
        match self.doc.draft.as_mut()? {
            Annotation::Path { points, .. }
            | Annotation::Mosaic {
                region: MosaicRegion::Path(points),
                ..
            }
            | Annotation::Eraser {
                region: MosaicRegion::Path(points),
                ..
            } => Some(points),
            _ => None,
        }
    }

    fn update_shape(&mut self, p: Pos) {
        // 永远从原始按下点计算，反向拖动时规范化的 min 不能成为新锚点。
        match self.doc.draft.as_mut() {
            Some(Annotation::Rect { rect, .. } | Annotation::Ellipse { rect, .. })
            | Some(
                Annotation::Mosaic {
                    region: MosaicRegion::Rect(rect),
                    ..
                }
                | Annotation::Eraser {
                    region: MosaicRegion::Rect(rect),
                    ..
                },
            ) => {
                *rect = Rect::from_points(self.anchor, p);
            }
            _ => self.doc.update_draft_end(p),
        }
    }

    /// 放弃尚未提交的草稿，不影响已有标注及历史。
    pub fn cancel_drag(&mut self) {
        // 拖到一半按 Esc 新建选区：`extend` 已经把 `doc.selection` 改成
        // 半成品了，这里还原回按下前的那个，让 Esc 的语义保持"什么都没发生"。
        if let Drag::NewSelection { previous, .. } = self.drag {
            self.doc.selection = previous;
        }
        if let Drag::MoveAnnotation { index, start } = self.drag {
            if let Some(item) = self.doc.items.get_mut(index) {
                item.translate(start.x - self.last.x, start.y - self.last.y);
            }
        }
        self.drag = Drag::Idle;
        self.doc.cancel_draft();
    }

    // ------------------------------------------------------------ 命令

    pub fn undo(&mut self) -> bool {
        self.cancel_drag();
        self.selected_item = None;
        self.history.undo(&mut self.doc).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.cancel_drag();
        self.selected_item = None;
        self.history.redo(&mut self.doc).is_some()
    }

    pub fn clear_annotations(&mut self) -> bool {
        self.cancel_drag();
        self.selected_item = None;
        match self.doc.clear_annotations() {
            Some(edit) => {
                self.history.record(edit);
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    fn constrained_endpoint(&self, p: Pos) -> Pos {
        if !self.constrain {
            return p;
        }
        let delta = p - self.anchor;
        match self.drag {
            Drag::NewSelection { .. } | Drag::Shape if self.doc.tool != Tool::Arrow => {
                let length = delta.x.abs().max(delta.y.abs());
                self.anchor + Pos::new(length * delta.x.signum(), length * delta.y.signum())
            }
            Drag::Shape if self.doc.tool == Tool::Arrow => {
                let angle = (delta.y.atan2(delta.x) / std::f32::consts::FRAC_PI_4).round()
                    * std::f32::consts::FRAC_PI_4;
                let length = (delta.x * delta.x + delta.y * delta.y).sqrt();
                self.anchor + Pos::new(length * angle.cos(), length * angle.sin())
            }
            _ => p,
        }
    }

    pub fn delete_selected(&mut self) {
        self.cancel_drag();
        if let Some(index) = self.selected_item.take() {
            if let Some(edit) = self.doc.erase_indices(vec![index]) {
                self.history.record(edit);
            }
        }
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    // ------------------------------------------------------------ 只读视图

    pub fn annotation_count(&self) -> usize {
        self.doc.items.len()
    }

    pub fn is_dragging(&self) -> bool {
        self.drag != Drag::Idle
    }

    pub fn show_magnifier(&self, alt: bool) -> bool {
        self.qr.is_none()
            && self.text.is_none()
            && (alt
                || self.is_snapping()
                || self.doc.selection.is_none()
                || (self.is_dragging() && self.doc.tool == Tool::Select))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_numbering_follows_undo_redo_deletion_and_new_sessions() {
        let mut ed = Editor::new();
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);
        ed.reset_session(screen, false);
        ed.set_selection(Some(Rect::new(100.0, 100.0, 500.0, 300.0)));
        ed.set_tool(Tool::Sequence);
        ed.add_sequence(Pos::new(200.0, 200.0));
        ed.add_sequence(Pos::new(300.0, 200.0));
        assert_eq!(ed.next_sequence_number(), 3);
        assert!(ed.undo());
        assert_eq!(ed.next_sequence_number(), 2);
        assert!(ed.redo());
        assert_eq!(ed.next_sequence_number(), 3);
        ed.selected_item = Some(1);
        ed.delete_selected();
        assert_eq!(ed.next_sequence_number(), 2);
        assert!(ed.undo());
        assert!(ed.undo());
        ed.add_sequence(Pos::new(400.0, 200.0));
        assert!(!ed.can_redo());
        assert!(matches!(
            ed.doc.items[1],
            Annotation::Sequence { number: 2, .. }
        ));
        ed.sequence_size = 54.0;
        ed.reset_session(screen, false);
        assert_eq!(ed.next_sequence_number(), 1);
        assert_eq!(ed.sequence_size, 54.0);
    }

    #[test]
    fn sequence_can_be_moved_deleted_and_restored_without_bitmap_history() {
        let mut ed = Editor::new();
        ed.set_selection(Some(Rect::new(-500.0, -200.0, 400.0, 300.0)));
        let center = Pos::new(-350.0, -50.0);
        ed.add_sequence(Pos::ZERO);
        assert!(ed.doc.items.is_empty());
        ed.add_sequence(center);
        let original = ed.doc.items[0].clone();
        assert!(original.hit_test(center, 0.0));
        assert!(!original.hit_test(center + Pos::new(15.0, 15.0), 0.0));
        ed.set_tool(Tool::Select);
        ed.begin(center);
        ed.finish(center + Pos::new(30.0, 20.0));
        assert!(
            matches!(ed.doc.items[0], Annotation::Sequence { center: p, number: 1, .. } if p == center + Pos::new(30.0, 20.0))
        );
        assert!(ed.undo());
        assert_eq!(ed.doc.items[0], original);
        ed.selected_item = Some(0);
        ed.delete_selected();
        assert!(ed.doc.items.is_empty());
        assert!(ed.undo());
        assert_eq!(ed.doc.items[0], original);
    }

    fn rect_editor() -> Editor {
        let mut ed = Editor::new();
        ed.set_tool(Tool::Rect);
        ed
    }

    #[test]
    fn drag_creates_one_rect_and_records_one_edit() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(10.0, 10.0));
        ed.extend(Pos::new(60.0, 40.0));
        ed.finish(Pos::new(100.0, 80.0));

        assert_eq!(ed.annotation_count(), 1);
        assert_eq!(ed.history.undo_len(), 1);
        assert!(ed.can_undo());
    }

    #[test]
    fn click_without_drag_produces_nothing() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(10.0, 10.0));
        ed.finish(Pos::new(10.0, 10.0));

        // 0×0 的矩形是"无意义草稿"，不应该污染撤销栈
        assert_eq!(ed.annotation_count(), 0);
        assert_eq!(ed.history.undo_len(), 0);
    }

    #[test]
    fn undo_then_redo_restores_the_rectangle() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(0.0, 0.0));
        ed.finish(Pos::new(20.0, 20.0));

        assert!(ed.undo());
        assert_eq!(ed.annotation_count(), 0);
        assert!(ed.can_redo());

        assert!(ed.redo());
        assert_eq!(ed.annotation_count(), 1);
    }

    #[test]
    fn dragging_backwards_still_yields_normalized_rect() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(100.0, 100.0));
        ed.finish(Pos::new(20.0, 30.0));

        let Annotation::Rect { rect, .. } = &ed.doc.items[0] else {
            panic!("应当是矩形");
        };
        assert_eq!(rect.min, Pos::new(20.0, 30.0));
        assert_eq!(rect.max, Pos::new(100.0, 100.0));
    }

    #[test]
    fn freehand_sampling_thins_dense_input() {
        let mut ed = Editor::new();
        ed.set_tool(Tool::Pen);
        ed.begin(Pos::new(0.0, 0.0));
        // 灌 200 个几乎重合的点
        for i in 0..200 {
            ed.extend(Pos::new(i as f32 * 0.01, 0.0));
        }
        ed.finish(Pos::new(2.0, 0.0));

        let Annotation::Path { points, .. } = &ed.doc.items[0] else {
            panic!("应当是路径");
        };
        assert!(
            points.len() < 30,
            "采样点应被大幅抽稀，实际 {} 个",
            points.len()
        );
    }

    #[test]
    fn freehand_draft_lives_until_release() {
        let mut ed = Editor::new();
        ed.set_tool(Tool::Pen);
        ed.begin(Pos::new(0.0, 0.0));
        ed.extend(Pos::new(10.0, 0.0));
        ed.extend(Pos::new(20.0, 0.0));

        // 松手前是草稿，不是已提交图元
        assert_eq!(ed.annotation_count(), 0);
        assert!(ed.doc.draft.is_some());

        ed.finish(Pos::new(30.0, 0.0));
        assert_eq!(ed.annotation_count(), 1);
        assert!(ed.doc.draft.is_none());
    }

    #[test]
    fn cancel_discards_draft_without_touching_history() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(0.0, 0.0));
        ed.extend(Pos::new(50.0, 50.0));
        ed.cancel_drag();

        assert_eq!(ed.annotation_count(), 0);
        assert_eq!(ed.history.undo_len(), 0);
        assert!(ed.doc.draft.is_none());
    }

    #[test]
    fn eraser_keeps_uncovered_annotations_and_undo_redo_as_one_stroke() {
        let mut ed = rect_editor();
        for x in [0.0, 100.0, 200.0] {
            ed.begin(Pos::new(x, 0.0));
            ed.finish(Pos::new(x + 50.0, 50.0));
        }
        let original = ed.doc.items.clone();
        ed.set_tool(Tool::Eraser);
        ed.begin(Pos::new(-10.0, 25.0));
        ed.extend(Pos::new(110.0, 25.0));
        ed.finish(Pos::new(260.0, 25.0));
        assert_eq!(&ed.doc.items[..3], &original);
        assert_eq!(ed.history.undo_len(), 4);
        assert!(matches!(ed.doc.items[3], Annotation::Eraser { .. }));
        assert!(ed.undo());
        assert_eq!(ed.doc.items, original);
        assert!(ed.redo());
        assert_eq!(ed.doc.items.len(), 4);
    }

    #[test]
    fn eraser_honours_a_fast_sweep_and_single_click() {
        let mut ed = Editor::new();
        ed.set_tool(Tool::Eraser);
        ed.begin(Pos::new(10.0, 25.0));
        ed.finish(Pos::new(160.0, 25.0));
        let stroke = &ed.doc.items[0];
        assert!(stroke.hit_test(Pos::new(85.0, 25.0), 0.0));
        assert!(!stroke.hit_test(Pos::new(85.0, 50.0), 0.0));
        ed.begin(Pos::new(50.0, 60.0));
        ed.finish(Pos::new(50.0, 60.0));
        assert_eq!(ed.doc.items.len(), 2);
        assert!(ed.doc.items[1].hit_test(Pos::new(50.0, 60.0), 0.0));
    }

    #[test]
    fn cancel_erase_discards_only_the_unfinished_stroke() {
        let mut ed = rect_editor();
        ed.begin(Pos::ZERO);
        ed.finish(Pos::new(50.0, 50.0));
        let original = ed.doc.items.clone();
        ed.set_tool(Tool::Eraser);
        ed.begin(Pos::new(-10.0, 25.0));
        ed.extend(Pos::new(60.0, 25.0));
        ed.cancel_drag();
        assert_eq!(ed.doc.items, original);
        assert_eq!(ed.history.undo_len(), 1);
        assert!(ed.doc.draft.is_none());
    }

    #[test]
    fn mosaic_rectangle_updates_in_both_drag_directions_and_is_undoable() {
        for end in [Pos::new(180.0, 170.0), Pos::new(20.0, 30.0)] {
            let mut ed = Editor::new();
            ed.set_tool(Tool::Mosaic);
            let anchor = Pos::new(100.0, 100.0);
            ed.begin(anchor);
            ed.extend(end);
            ed.finish(end);
            assert!(matches!(&ed.doc.items[0], Annotation::Mosaic {
                region: MosaicRegion::Rect(r), ..
            } if *r == Rect::from_points(anchor, end)));
            assert!(ed.undo());
            assert!(ed.doc.items.is_empty());
            assert!(ed.redo());
        }
    }

    #[test]
    fn switching_tool_drops_the_draft() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(0.0, 0.0));
        ed.extend(Pos::new(30.0, 30.0));
        ed.set_tool(Tool::Ellipse);

        assert!(ed.doc.draft.is_none());
        assert_eq!(ed.annotation_count(), 0);
    }

    #[test]
    fn select_tool_moves_selection() {
        let mut ed = Editor::new();
        ed.set_selection(Some(Rect::new(100.0, 100.0, 200.0, 100.0)));
        ed.set_tool(Tool::Select);

        ed.begin(Pos::new(150.0, 150.0));
        ed.extend(Pos::new(170.0, 160.0));
        ed.finish(Pos::new(170.0, 160.0));

        let sel = ed.doc.selection.unwrap();
        assert_eq!(sel.min, Pos::new(120.0, 110.0));
        assert_eq!(sel.max, Pos::new(320.0, 210.0));
        // 移动选区不是文档编辑，不进撤销栈
        assert_eq!(ed.history.undo_len(), 0);
    }

    #[test]
    fn select_full_screen_matches_the_screen_exactly() {
        // 兜底选区必须**就是**整屏，不能再做比例内缩 ——
        // 用户要的是"自适应当前桌面"，不是"屏幕中间 70%"
        let mut ed = Editor::new();
        ed.select_full_screen(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        assert_eq!(
            ed.doc.selection.unwrap(),
            Rect::new(0.0, 0.0, 1000.0, 1000.0)
        );
    }

    #[test]
    fn dragging_inside_a_full_screen_selection_starts_a_new_one() {
        // 整屏选区只是"用户还没选"的占位。若把它当成真选区做平移，
        // 用户会得到"按下去什么都不发生"——平移一个铺满屏幕的矩形，
        // 屏幕上根本看不出区别。正确行为是重新框选。
        let mut ed = Editor::new();
        ed.set_screen_bounds(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        ed.select_full_screen(Rect::new(0.0, 0.0, 1000.0, 1000.0));

        ed.begin(Pos::new(300.0, 300.0));
        ed.finish(Pos::new(700.0, 700.0));

        let sel = ed.doc.selection.expect("选区不该消失");
        assert_eq!(sel.min, Pos::new(300.0, 300.0));
        assert_eq!(sel.max, Pos::new(700.0, 700.0));
    }

    #[test]
    fn a_window_selection_is_still_moved_not_rebuilt() {
        // 与上一条互为边界：真正的窗口选区（不是整屏）内部按下仍是平移
        let mut ed = selection_editor();
        ed.begin(Pos::new(400.0, 350.0));
        ed.finish(Pos::new(450.0, 400.0));

        let sel = ed.doc.selection.expect("选区不该消失");
        assert_eq!(sel.width(), 400.0, "平移不该改变尺寸");
        assert_eq!(sel.min, Pos::new(250.0, 250.0));
    }

    #[test]
    fn select_window_clips_to_the_screen() {
        // 窗口有一半在屏幕外：选区应被裁到交集，而不是露出一截空白
        let mut ed = Editor::new();
        let screen = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        ed.select_window(Rect::new(800.0, 100.0, 400.0, 300.0), screen);
        assert_eq!(
            ed.doc.selection.unwrap(),
            Rect::new(800.0, 100.0, 200.0, 300.0)
        );
    }

    // ------------------------------------------------------------ 选区调整

    /// 造一个选区 + 指针工具，并把可选区边界设成 1000×1000。
    fn selection_editor() -> Editor {
        let mut ed = Editor::new();
        // ⚠️ 刻意**不**在这里设置工具 —— 必须走程序启动时的真实默认值。
        //
        // 这里原先写的是 `ed.set_tool(Tool::Select)`，于是"`Document::new()`
        // 的默认工具其实是 `Tool::Rect`"这个真实缺陷被完全盖住：所有角柄
        // 测试都在一个用户根本遇不到的状态下跑，全绿，而线上按 F1 后拖角柄
        // 只会画出一个红矩形。
        assert_eq!(
            ed.doc.tool,
            Tool::Select,
            "截图工具的默认工具必须是指针：第一步永远是先选区域，不是先画图"
        );
        ed.set_screen_bounds(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        ed.set_selection(Some(Rect::new(200.0, 200.0, 400.0, 300.0)));
        ed
    }

    #[test]
    fn dragging_a_corner_handle_resizes_the_selection() {
        let mut ed = selection_editor();
        // 右下角在 (600, 500)
        ed.begin(Pos::new(600.0, 500.0));
        ed.extend(Pos::new(700.0, 650.0));
        ed.finish(Pos::new(700.0, 650.0));

        let sel = ed.doc.selection.unwrap();
        assert_eq!(sel.min, Pos::new(200.0, 200.0), "左上角不该动");
        assert_eq!(sel.max, Pos::new(700.0, 650.0));
        // 调选区不是文档编辑，不该污染撤销栈
        assert_eq!(ed.history.undo_len(), 0);
    }

    #[test]
    fn dragging_an_edge_handle_moves_only_that_edge() {
        let mut ed = selection_editor();
        // 右边中点 (600, 350)
        ed.begin(Pos::new(600.0, 350.0));
        ed.finish(Pos::new(750.0, 350.0));

        let sel = ed.doc.selection.unwrap();
        assert_eq!(sel.min, Pos::new(200.0, 200.0));
        assert_eq!(sel.max, Pos::new(750.0, 500.0), "上下边不应被带动");
    }

    #[test]
    fn dragging_inside_moves_the_whole_selection() {
        let mut ed = selection_editor();
        // 选区中心附近，远离任何角柄
        ed.begin(Pos::new(400.0, 350.0));
        ed.extend(Pos::new(430.0, 380.0));
        ed.finish(Pos::new(430.0, 380.0));

        let sel = ed.doc.selection.unwrap();
        assert_eq!(sel.min, Pos::new(230.0, 230.0));
        assert_eq!(sel.max, Pos::new(630.0, 530.0), "尺寸必须保持不变");
    }

    #[test]
    fn dragging_outside_starts_a_brand_new_selection() {
        let mut ed = selection_editor();
        ed.begin(Pos::new(50.0, 50.0));
        ed.extend(Pos::new(150.0, 120.0));
        ed.finish(Pos::new(150.0, 120.0));

        assert_eq!(
            ed.doc.selection.unwrap(),
            Rect::new(50.0, 50.0, 100.0, 70.0)
        );
    }

    #[test]
    fn a_stray_click_outside_keeps_the_old_selection() {
        let mut ed = selection_editor();
        let before = ed.doc.selection.unwrap();

        // 点一下就走，没拖出任何面积
        ed.begin(Pos::new(50.0, 50.0));
        ed.finish(Pos::new(51.0, 51.0));

        assert_eq!(ed.doc.selection.unwrap(), before, "误点不该清掉选区");
    }

    #[test]
    fn resizing_past_the_opposite_edge_stops_at_the_minimum() {
        let mut ed = selection_editor();
        // 抓左边中点，一路拖到右边界的右侧
        ed.begin(Pos::new(200.0, 350.0));
        ed.finish(Pos::new(9999.0, 350.0));

        let sel = ed.doc.selection.unwrap();
        assert_eq!(sel.width(), MIN_SELECTION_SIZE);
        assert_eq!(sel.max.x, 600.0, "右边界应原地不动");
    }

    #[test]
    fn selection_cannot_be_dragged_off_screen() {
        let mut ed = selection_editor();
        // 抓着选区往右上角外面拖
        ed.begin(Pos::new(400.0, 350.0));
        ed.finish(Pos::new(5000.0, -5000.0));

        let sel = ed.doc.selection.unwrap();
        assert!(sel.max.x <= 1000.0, "右边界越界了：{sel:?}");
        assert!(sel.min.y >= 0.0, "上边界越界了：{sel:?}");
        // 而且尺寸不变 —— 是"被挡住"，不是"被压扁"
        assert_eq!(sel.size(), (400.0, 300.0));
    }

    #[test]
    fn cancel_during_new_selection_restores_the_previous_one() {
        let mut ed = selection_editor();
        let before = ed.doc.selection.unwrap();

        ed.begin(Pos::new(50.0, 50.0));
        ed.extend(Pos::new(150.0, 150.0));
        assert_ne!(ed.doc.selection.unwrap(), before, "拖拽中应已改变");

        ed.cancel_drag();
        assert_eq!(ed.doc.selection.unwrap(), before, "Esc 应完全还原");
    }

    // ------------------------------------------------------------ 窗口吸附

    #[test]
    fn snap_mode_disarms_as_soon_as_the_user_drags() {
        let mut ed = selection_editor();
        ed.arm_snap();
        assert!(ed.is_snapping());

        ed.begin(Pos::new(400.0, 350.0));
        assert!(!ed.is_snapping(), "用户一旦动手，就不能再被自动吸附改回去");
    }

    #[test]
    fn snap_probe_debounces_small_pointer_moves() {
        let mut ed = selection_editor();
        ed.arm_snap();

        assert!(ed.take_snap_probe(Pos::new(100.0, 100.0)), "首次应探测");
        assert!(
            !ed.take_snap_probe(Pos::new(102.0, 101.0)),
            "小幅抖动不该反复查询窗口"
        );
        assert!(
            ed.take_snap_probe(Pos::new(200.0, 100.0)),
            "移动足够远后应重新探测"
        );
    }

    #[test]
    fn snap_probe_is_off_when_not_snapping() {
        let mut ed = selection_editor();
        assert!(!ed.take_snap_probe(Pos::new(1.0, 1.0)));
    }

    #[test]
    fn clear_annotations_is_undoable() {
        let mut ed = rect_editor();
        ed.begin(Pos::new(0.0, 0.0));
        ed.finish(Pos::new(40.0, 40.0));
        ed.begin(Pos::new(60.0, 60.0));
        ed.finish(Pos::new(90.0, 90.0));
        assert_eq!(ed.annotation_count(), 2);

        assert!(ed.clear_annotations());
        assert_eq!(ed.annotation_count(), 0);

        assert!(ed.undo());
        assert_eq!(ed.annotation_count(), 2);
    }

    #[test]
    fn annotation_move_and_delete_are_undoable_and_drag_cancel_restores_position() {
        let mut ed = selection_editor();
        ed.doc.items.push(Annotation::Rect {
            rect: Rect::new(300.0, 300.0, 60.0, 40.0),
            style: Style::default(),
        });
        let before = ed.doc.items[0].clone();
        let selection = ed.doc.selection;
        ed.begin(Pos::new(300.0, 310.0));
        ed.extend(Pos::new(330.0, 330.0));
        ed.cancel_drag();
        assert_eq!(ed.doc.items[0], before);
        ed.begin(Pos::new(300.0, 310.0));
        ed.finish(Pos::new(330.0, 330.0));
        assert_eq!(ed.doc.selection, selection);
        assert_ne!(ed.doc.items[0], before);
        assert!(ed.undo());
        assert_eq!(ed.doc.items[0], before);
        assert!(ed.redo());
        ed.selected_item = Some(0);
        ed.delete_selected();
        assert!(ed.doc.items.is_empty());
        assert!(ed.undo());
        assert_eq!(ed.doc.items.len(), 1);
    }

    #[test]
    fn new_session_clears_document_and_shift_constrains_geometry() {
        let mut ed = rect_editor();
        ed.constrain = true;
        ed.begin(Pos::new(20.0, 20.0));
        ed.finish(Pos::new(60.0, 40.0));
        match &ed.doc.items[0] {
            Annotation::Rect { rect, .. } => assert_eq!(rect.width(), rect.height()),
            _ => panic!("应为矩形"),
        }
        ed.reset_session(Rect::new(0.0, 0.0, 1000.0, 800.0), true);
        assert!(ed.doc.items.is_empty());
        assert!(!ed.can_undo());
        assert!(ed.is_snapping());
        assert_eq!(ed.doc.tool, Tool::Select);
    }
}
