//! 待导出的文档模型：选区 + 图元列表 + 当前草稿。
//!
//! ## 编辑模式约定：先应用，再记录
//!
//! `Document` 上的修改方法（[`Document::commit_draft`]、[`Document::erase_indices`] 等）
//! 会**直接改动 `items`**，并返回一条描述了这次改动的 [`Edit`]。
//! 调用方拿到 `Edit` 后交给 [`crate::History::record`] 登记即可。
//!
//! 这样设计的好处是不会出现"改了但忘了记"或"记了但改了两次"的经典 bug ——
//! `History` 只在 `undo` / `redo` 时才真正调用 `apply` / `revert`。

use crate::annotation::Annotation;
use crate::geometry::{Pos, Rect};
use crate::history::Edit;
use crate::style::{Style, Tool};

/// 截图画布文档。
#[derive(Debug, Clone)]
pub struct Document {
    /// 用户框选出来的区域（VSS 物理像素）。`None` 表示尚未框选。
    pub selection: Option<Rect>,
    /// 已提交的标注图元，按 z 序从底到顶。
    pub items: Vec<Annotation>,
    /// 正在绘制中的临时图元。不参与渲染的"已提交"列表，也不进撤销栈。
    pub draft: Option<Annotation>,
    pub tool: Tool,
    pub style: Style,
    /// 正在编辑的文字图元索引。`Some` 时该图元的文字框会接收键盘与 IME 输入。
    pub editing_text: Option<usize>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            selection: None,
            items: Vec::new(),
            draft: None,
            // ⚠️ 默认必须是**指针**，不能是某个绘图工具。
            //
            // 截图工具的第一步永远是"确定截哪一块"，而不是"先画个矩形"。
            // 若默认成 `Tool::Rect`，用户按下 F1 看到选区、想拖角柄调整大小时，
            // 按下鼠标会变成**画一个红色矩形**——选区纹丝不动。这正是线上反馈
            // 的"不能自由调整截图的选择"。
            tool: Tool::Select,
            style: Style::default(),
            editing_text: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.draft.is_none()
    }

    /// 当选区不存在时，返回 `None`；导出流程必须先确保有选区。
    pub fn selection_rect(&self) -> Option<Rect> {
        self.selection.filter(|r| !r.is_empty())
    }

    /// 是否已经有可用于导出的内容（选区有效 且 没有"空"选区）。
    pub fn is_exportable(&self) -> bool {
        self.selection_rect().is_some()
    }

    /// 所有已提交图元的外接矩形并集。注意**不包含**选区自身。
    pub fn annotations_bounds(&self) -> Option<Rect> {
        self.items
            .iter()
            .map(|a| a.bounds())
            .reduce(|acc, r| acc.union(&r))
    }

    /// 导出内容的外接矩形 = 选区 ∪ 标注（但通常裁剪到选区）。
    pub fn content_bounds(&self) -> Option<Rect> {
        let sel = self.selection_rect()?;
        Some(match self.annotations_bounds() {
            Some(a) => sel.union(&a),
            None => sel,
        })
    }

    // ---------------------------------------------------------------- 绘制草稿

    /// 提交当前草稿。
    ///
    /// 空草稿（例如用户只是点了一下没拖动）会被丢弃并返回 `None`，
    /// 避免在撤销栈里塞入看不见的空图元。
    pub fn commit_draft(&mut self) -> Option<Edit> {
        let draft = self.draft.take()?;
        if !is_meaningful(&draft) {
            return None;
        }
        self.items.push(draft.clone());
        Some(Edit::Add { item: draft })
    }

    /// 丢弃当前草稿（`Esc` 取消绘制中）。
    ///
    /// 不产生 `Edit` —— 草稿从未进入 `items`，不属于历史的一部分。
    pub fn cancel_draft(&mut self) {
        self.draft = None;
    }

    /// 正在绘制时更新草稿的终点（矩形 / 椭圆 / 箭头）。
    pub fn update_draft_end(&mut self, end: Pos) {
        match self.draft.as_mut() {
            Some(Annotation::Rect { rect, .. }) | Some(Annotation::Ellipse { rect, .. }) => {
                *rect = Rect::from_points(rect.min, end);
            }
            Some(Annotation::Arrow { to, .. }) => *to = end,
            Some(Annotation::Mosaic { region, .. } | Annotation::Eraser { region, .. }) => {
                if let crate::annotation::MosaicRegion::Rect(rect) = region {
                    *rect = Rect::from_points(rect.min, end);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- 命中

    /// 找出 z 序最顶端的、被 `p` 命中的图元索引。
    ///
    /// 从后往前扫描 —— 后画的图元在上层，应当优先被点中。
    pub fn topmost_hit(&self, p: Pos, extra: f32) -> Option<(usize, &Annotation)> {
        self.items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, a)| a.hit_test(p, extra))
    }

    /// 收集给定采样点命中的图元索引（升序去重），用于对象选择。
    pub fn collect_hits(&self, trail: &[Pos], extra: f32) -> Vec<usize> {
        let mut hits: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, a)| trail.iter().any(|p| a.hit_test(*p, extra)))
            .map(|(i, _)| i)
            .collect();
        hits.sort_unstable();
        hits.dedup();
        hits
    }

    /// 删除指定索引的图元，返回可登记的 `Edit`。
    ///
    /// 索引会被过滤到合法范围；空集合返回 `None`。
    /// 批量对象删除只产生一条 `Edit::RemoveMany`。
    pub fn erase_indices(&mut self, mut indices: Vec<usize>) -> Option<Edit> {
        indices.retain(|i| *i < self.items.len());
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return None;
        }

        let mut entries: Vec<(usize, Annotation)> = Vec::with_capacity(indices.len());
        for &i in &indices {
            entries.push((i, self.items[i].clone()));
        }

        // 从后往前删，保证前面的索引仍然有效
        for &i in indices.iter().rev() {
            self.items.remove(i);
        }

        // 被删掉的图元若正在被编辑，结束编辑状态
        if let Some(editing) = self.editing_text {
            if indices.contains(&editing) {
                self.editing_text = None;
            }
        }

        if entries.len() == 1 {
            let (index, item) = entries.pop().expect("长度已判定为 1");
            Some(Edit::Remove { index, item })
        } else {
            Some(Edit::RemoveMany { entries })
        }
    }

    /// 清空全部标注（保留选区）。
    pub fn clear_annotations(&mut self) -> Option<Edit> {
        if self.items.is_empty() {
            return None;
        }
        let items = std::mem::take(&mut self.items);
        self.draft = None;
        self.editing_text = None;
        Some(Edit::Clear { items })
    }

    /// 平移若干图元（拖动已有标注）。
    pub fn translate_items(&mut self, indices: &[usize], dx: f32, dy: f32) -> Option<Edit> {
        let valid: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|i| *i < self.items.len())
            .collect();
        if valid.is_empty() {
            return None;
        }
        for &i in &valid {
            self.items[i].translate(dx, dy);
        }
        Some(Edit::Translate {
            indices: valid,
            dx,
            dy,
        })
    }

    /// 切换工具，同时结束正在进行中的文字编辑。
    ///
    /// 文字编辑状态必须显式结束，否则切走工具后输入框会"看不见但还在吃键盘事件"。
    pub fn set_tool(&mut self, tool: Tool) {
        if self.tool != tool {
            self.tool = tool;
            self.draft = None;
            self.editing_text = None;
        }
    }
}

/// 判断一个草稿是否"有意义"（值得进撤销栈）。
///
/// 规则：有实际尺寸的图元才算数。用户单击一下（0×0 的矩形）不产生图元，
/// 这符合直觉，也避免撤销栈被垃圾填满。
fn is_meaningful(a: &Annotation) -> bool {
    match a {
        Annotation::Rect { rect, .. } | Annotation::Ellipse { rect, .. } => !rect.is_empty(),
        Annotation::Arrow { from, to, .. } => from.distance(*to) >= 2.0,
        Annotation::Path { points, .. } => points.len() >= 2,
        Annotation::Mosaic { region, .. } | Annotation::Eraser { region, .. } => match region {
            crate::annotation::MosaicRegion::Rect(r) => !r.is_empty(),
            crate::annotation::MosaicRegion::Path(pts) => !pts.is_empty(),
        },
        Annotation::Text { content, .. } => !content.trim().is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::{ArrowKind, PathKind};
    use crate::style::Color;

    fn doc_with_rect(x: f32, y: f32) -> Document {
        let mut d = Document::new();
        d.items.push(Annotation::Rect {
            rect: Rect::new(x, y, 100.0, 100.0),
            style: Style::default(),
        });
        d
    }

    #[test]
    fn empty_draft_is_discarded() {
        let mut d = Document::new();
        // 用户只是点了一下：0×0 的矩形
        d.draft = Some(Annotation::Rect {
            rect: Rect::new(10.0, 10.0, 0.0, 0.0),
            style: Style::default(),
        });
        assert!(d.commit_draft().is_none());
        assert!(d.items.is_empty(), "空草稿不应进入 items");
    }

    #[test]
    fn meaningful_draft_is_committed() {
        let mut d = Document::new();
        d.draft = Some(Annotation::Rect {
            rect: Rect::new(10.0, 10.0, 50.0, 50.0),
            style: Style::default(),
        });
        let edit = d.commit_draft().expect("应产生 Edit");
        assert!(matches!(edit, Edit::Add { .. }));
        assert_eq!(d.items.len(), 1);
        assert!(d.draft.is_none());
    }

    #[test]
    fn tiny_arrow_is_discarded() {
        let mut d = Document::new();
        d.draft = Some(Annotation::Arrow {
            from: Pos::new(10.0, 10.0),
            to: Pos::new(10.5, 10.5), // 距离 < 2.0
            kind: ArrowKind::Straight,
            style: Style::default(),
        });
        assert!(d.commit_draft().is_none());
    }

    #[test]
    fn single_point_path_is_discarded() {
        let mut d = Document::new();
        d.draft = Some(Annotation::Path {
            points: vec![Pos::new(1.0, 1.0)],
            kind: PathKind::Pen,
            style: Style::default(),
        });
        assert!(d.commit_draft().is_none());
    }

    #[test]
    fn topmost_hit_prefers_later_item() {
        let mut d = doc_with_rect(0.0, 0.0);
        d.items.push(Annotation::Rect {
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            style: Style::default(),
        });
        // 两个矩形边框完全重合，应命中后加入的那个（索引 1）
        let (idx, _) = d.topmost_hit(Pos::new(0.0, 50.0), 0.0).unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn erase_indices_removes_and_builds_edit() {
        let mut d = Document::new();
        for i in 0..4 {
            d.items.push(Annotation::Rect {
                rect: Rect::new(i as f32 * 200.0, 0.0, 100.0, 100.0),
                style: Style::default(),
            });
        }
        // 删掉索引 1 与 3
        let edit = d.erase_indices(vec![3, 1]).expect("应有 Edit");
        assert_eq!(d.items.len(), 2);

        // 剩下的应当是原来的 0 与 2（bounds 含线宽 3.0 的一半外扩）
        for (i, item) in d.items.iter().enumerate() {
            let expected_x = if i == 0 { 0.0 } else { 400.0 };
            assert_eq!(item.bounds().min.x, expected_x - 1.5);
        }

        // 多选删除应产生 RemoveMany，而不是多条 Remove
        match edit {
            Edit::RemoveMany { ref entries } => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].0, 1);
                assert_eq!(entries[1].0, 3);
            }
            other => panic!("期望 RemoveMany, 得到 {other:?}"),
        }
    }

    #[test]
    fn erase_single_produces_remove() {
        let mut d = doc_with_rect(0.0, 0.0);
        let edit = d.erase_indices(vec![0]).unwrap();
        assert!(matches!(edit, Edit::Remove { index: 0, .. }));
        assert!(d.items.is_empty());
    }

    #[test]
    fn erase_out_of_range_is_ignored() {
        let mut d = doc_with_rect(0.0, 0.0);
        assert!(d.erase_indices(vec![99]).is_none());
        assert_eq!(d.items.len(), 1);
        assert!(d.erase_indices(vec![]).is_none());
    }

    #[test]
    fn collect_hits_dedups_and_sorts() {
        let mut d = Document::new();
        for i in 0..3 {
            d.items.push(Annotation::Rect {
                rect: Rect::new(0.0, i as f32 * 50.0, 100.0, 40.0),
                style: Style::default(),
            });
        }
        // 一条竖直的橡皮擦轨迹，会依次划过三个矩形
        let trail = [
            Pos::new(0.0, 0.0),
            Pos::new(0.0, 25.0),
            Pos::new(0.0, 60.0),
            Pos::new(0.0, 110.0),
        ];
        let hits = d.collect_hits(&trail, 0.0);
        assert_eq!(hits, vec![0, 1, 2]);
    }

    #[test]
    fn clear_annotations_captures_items_for_undo() {
        let mut d = doc_with_rect(0.0, 0.0);
        d.items.push(Annotation::Rect {
            rect: Rect::new(200.0, 0.0, 10.0, 10.0),
            style: Style::default(),
        });
        let edit = d.clear_annotations().unwrap();
        assert!(d.items.is_empty());
        match edit {
            Edit::Clear { items } => assert_eq!(items.len(), 2),
            other => panic!("期望 Clear, 得到 {other:?}"),
        }
        // 已经空了 → 不再产生 Edit，避免撤销栈出现无效记录
        assert!(d.clear_annotations().is_none());
    }

    #[test]
    fn erase_clears_text_editing_state() {
        let mut d = Document::new();
        d.items.push(Annotation::Text {
            rect: Rect::new(0.0, 0.0, 50.0, 20.0),
            content: "abc".into(),
            style: Style::default(),
        });
        d.editing_text = Some(0);
        d.erase_indices(vec![0]);
        assert!(
            d.editing_text.is_none(),
            "删除正在编辑的文字后必须退出编辑态"
        );
    }

    #[test]
    fn translate_items_records_and_skips_invalid() {
        let mut d = doc_with_rect(0.0, 0.0);
        let edit = d.translate_items(&[0, 5], 10.0, 20.0).unwrap();
        match edit {
            Edit::Translate { indices, dx, dy } => {
                assert_eq!(indices, vec![0], "非法索引应被过滤");
                assert_eq!((dx, dy), (10.0, 20.0));
            }
            other => panic!("期望 Translate, 得到 {other:?}"),
        }
        // 默认线宽 3.0 → bounds 四周外扩 1.5
        assert_eq!(d.items[0].bounds().min.x, 8.5);
    }

    #[test]
    fn switching_tool_cancels_draft_and_editing() {
        let mut d = Document::new();
        d.tool = Tool::Rect;
        d.draft = Some(Annotation::Rect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            style: Style::default(),
        });
        d.editing_text = Some(0);
        d.set_tool(Tool::Arrow);
        assert!(d.draft.is_none());
        assert!(d.editing_text.is_none());
        assert_eq!(d.tool, Tool::Arrow);
    }

    #[test]
    fn switching_to_same_tool_is_noop() {
        let mut d = Document::new();
        d.tool = Tool::Rect;
        d.draft = Some(Annotation::Rect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            style: Style::default(),
        });
        d.set_tool(Tool::Rect);
        assert!(d.draft.is_some(), "重复选择同一工具不应清空草稿");
    }

    #[test]
    fn exportable_requires_non_empty_selection() {
        let mut d = Document::new();
        assert!(!d.is_exportable());
        d.selection = Some(Rect::new(0.0, 0.0, 0.0, 0.0));
        assert!(!d.is_exportable(), "零面积选区不可导出");
        d.selection = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(d.is_exportable());
    }

    #[test]
    fn content_bounds_unions_selection_and_annotations() {
        let mut d = Document::new();
        d.selection = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        d.items.push(Annotation::Rect {
            rect: Rect::new(150.0, 0.0, 100.0, 100.0),
            style: Style {
                stroke: Color::rgb(0, 0, 0),
                stroke_width: 0.0,
                ..Style::default()
            },
        });
        let b = d.content_bounds().unwrap();
        assert_eq!(b.max.x, 250.0);
    }
}
