//! 撤销 / 重做栈。
//!
//! ## 为什么不是快照式撤销
//!
//! 快照式（每次操作前把整个 `items` 深拷贝一份）实现最简单，但代价是
//! 每画一笔就复制一次全部标注。100 步历史 + 100 个图元 = 上万次结构体拷贝，
//! 而且随着标注变多，每次快照的成本线性上升。
//!
//! 本模块只记录**操作本身**（`Edit`），撤销 = 施加逆操作。单条记录的体积
//! 恒定在几百字节，与画布内容规模无关。
//!
//! ## 约定
//!
//! [`Document`] 上的修改方法会**先应用、再返回 `Edit`**（见 `document` 模块文档）。
//! 因此 [`History::record`] 只入栈、不应用；`apply` / `revert` 仅在
//! [`History::undo`] / [`History::redo`] 内部被调用。

use std::collections::VecDeque;

use crate::annotation::Annotation;
use crate::document::Document;

/// 一次可撤销的编辑操作。
///
/// 所有变体都携带足以**完整还原**原始状态的数据 ——
/// 例如 `Remove` 把被删掉的图元本体也存了下来，否则撤销时无从恢复。
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// 新增一个图元（提交草稿）。
    Add { item: Annotation },
    /// 删除单个图元。
    Remove { index: usize, item: Annotation },
    /// 批量删除多个图元。`entries` 按索引**升序**排列。
    ///
    /// 合并成一条记录，是为了让"一次拖动 = 一次撤销"，而不是让用户按十几次 `Ctrl+Z`。
    RemoveMany { entries: Vec<(usize, Annotation)> },
    /// 平移若干图元。
    Translate {
        indices: Vec<usize>,
        dx: f32,
        dy: f32,
    },
    /// 清空全部标注。
    Clear { items: Vec<Annotation> },
}

impl Edit {
    /// 正向应用（重做时调用）。
    pub fn apply(&self, doc: &mut Document) {
        match self {
            Edit::Add { item } => doc.items.push(item.clone()),
            Edit::Remove { index, .. } => {
                if *index < doc.items.len() {
                    doc.items.remove(*index);
                }
            }
            Edit::RemoveMany { entries } => {
                // 从后往前删，保证剩余索引仍然有效
                for (index, _) in entries.iter().rev() {
                    if *index < doc.items.len() {
                        doc.items.remove(*index);
                    }
                }
            }
            Edit::Translate { indices, dx, dy } => {
                for &i in indices {
                    if let Some(item) = doc.items.get_mut(i) {
                        item.translate(*dx, *dy);
                    }
                }
            }
            Edit::Clear { .. } => doc.items.clear(),
        }
    }

    /// 反向还原（撤销时调用）。
    pub fn revert(&self, doc: &mut Document) {
        match self {
            Edit::Add { .. } => {
                // 撤销一次 Add 就是把它从末尾弹掉。
                //
                // 这里刻意不做"按值查找后删除"：`Annotation` 实现了 `PartialEq`，
                // 两个内容相同的图元无法区分，按值查找可能删错对象。
                // 而 Add 一定是被追加到末尾的，pop 是精确且 O(1) 的。
                doc.items.pop();
            }
            Edit::Remove { index, item } => {
                let at = (*index).min(doc.items.len());
                doc.items.insert(at, item.clone());
            }
            Edit::RemoveMany { entries } => {
                // 按升序插回，保证最终顺序与删除前完全一致
                for (index, item) in entries {
                    let at = (*index).min(doc.items.len());
                    doc.items.insert(at, item.clone());
                }
            }
            Edit::Translate { indices, dx, dy } => {
                for &i in indices {
                    if let Some(item) = doc.items.get_mut(i) {
                        item.translate(-*dx, -*dy);
                    }
                }
            }
            Edit::Clear { items } => doc.items = items.clone(),
        }
    }

    /// 用于提示条 / 调试面板的短描述。
    pub fn label(&self) -> &'static str {
        match self {
            Edit::Add { .. } => "添加标注",
            Edit::Remove { .. } => "删除标注",
            Edit::RemoveMany { .. } => "删除多个标注",
            Edit::Translate { .. } => "移动标注",
            Edit::Clear { .. } => "清空标注",
        }
    }

    /// 估算这条记录占用的近似字节数（仅用于调试面板展示内存占用）。
    pub fn approx_bytes(&self) -> usize {
        use std::mem::size_of;
        size_of::<Self>()
            + match self {
                Edit::Add { .. } | Edit::Remove { .. } => 0,
                Edit::RemoveMany { entries } => entries.len() * size_of::<(usize, Annotation)>(),
                Edit::Translate { indices, .. } => indices.len() * size_of::<usize>(),
                Edit::Clear { items } => items.len() * size_of::<Annotation>(),
            }
    }
}

/// 撤销 / 重做栈。
#[derive(Debug, Clone)]
pub struct History {
    undo_stack: VecDeque<Edit>,
    redo_stack: Vec<Edit>,
    capacity: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(50)
    }
}

impl History {
    /// 默认保留 50 步。
    ///
    /// 上限并非出于内存考虑（单条记录只有几百字节），而是防止极端情况下
    /// 用户翻出一小时前的操作造成语义混乱。
    pub const DEFAULT_CAPACITY: usize = 50;

    pub fn new(capacity: usize) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// 登记一条**已经应用过**的编辑。
    ///
    /// 任何新的操作都必须清空重做栈 —— 这是所有编辑器的一致语义：
    /// 撤销之后又画了一笔，原本可重做的分支就作废了。
    pub fn record(&mut self, edit: Edit) {
        self.redo_stack.clear();
        self.undo_stack.push_back(edit);
        // 用 VecDeque 从前端淘汰最旧的记录，O(1) 而不是 Vec 的 O(n) 搬移
        while self.undo_stack.len() > self.capacity {
            self.undo_stack.pop_front();
        }
    }

    /// 撤销一步。返回被撤销的操作描述。
    pub fn undo(&mut self, doc: &mut Document) -> Option<&Edit> {
        let edit = self.undo_stack.pop_back()?;
        edit.revert(doc);
        self.redo_stack.push(edit);
        self.redo_stack.last()
    }

    /// 重做一步。返回被重做的操作描述。
    pub fn redo(&mut self, doc: &mut Document) -> Option<&Edit> {
        let edit = self.redo_stack.pop()?;
        edit.apply(doc);
        self.undo_stack.push_back(edit);
        // VecDeque 的"末尾"是 back()，last() 只存在于迭代器上
        self.undo_stack.back()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 下一次撤销会做什么（用于工具栏按钮的 tooltip）。
    pub fn peek_undo_label(&self) -> Option<&'static str> {
        self.undo_stack.back().map(|e| e.label())
    }

    pub fn peek_redo_label(&self) -> Option<&'static str> {
        self.redo_stack.last().map(|e| e.label())
    }

    /// 全部清空。开始新一次截图时必须调用，否则会串到上一次的历史。
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// 估算整个历史占用的近似字节数。
    pub fn approx_bytes(&self) -> usize {
        self.undo_stack
            .iter()
            .map(|e| e.approx_bytes())
            .sum::<usize>()
            + self
                .redo_stack
                .iter()
                .map(|e| e.approx_bytes())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::{ArrowKind, PathKind};
    use crate::geometry::{Pos, Rect};
    use crate::style::Style;

    fn rect_at(x: f32) -> Annotation {
        Annotation::Rect {
            rect: Rect::new(x, 0.0, 100.0, 100.0),
            style: Style::default(),
        }
    }

    /// 便利函数：模拟"应用 + 记录"的两步操作。
    fn record_add(h: &mut History, doc: &mut Document, x: f32) {
        doc.items.push(rect_at(x));
        h.record(Edit::Add { item: rect_at(x) });
    }

    #[test]
    fn undo_and_redo_add() {
        let mut doc = Document::new();
        let mut h = History::default();

        record_add(&mut h, &mut doc, 0.0);
        record_add(&mut h, &mut doc, 200.0);
        assert_eq!(doc.items.len(), 2);

        h.undo(&mut doc);
        assert_eq!(doc.items.len(), 1);
        assert!(h.can_redo());

        h.redo(&mut doc);
        assert_eq!(doc.items.len(), 2);
        assert!(!h.can_redo());
    }

    #[test]
    fn undo_restores_deleted_item_at_original_index() {
        let mut doc = Document::new();
        for i in 0..3 {
            doc.items.push(rect_at(i as f32 * 200.0));
        }
        let mut h = History::default();

        // 删掉中间那个
        let edit = doc.erase_indices(vec![1]).unwrap();
        h.record(edit);
        assert_eq!(doc.items.len(), 2);

        h.undo(&mut doc);
        assert_eq!(doc.items.len(), 3);
        // 必须插回原位置，而不是追加到末尾 —— 否则 z 序就乱了
        assert_eq!(doc.items[1].bounds().min.x, 200.0 - 1.5);
    }

    #[test]
    fn removemany_roundtrip_preserves_order() {
        let mut doc = Document::new();
        for i in 0..5 {
            doc.items.push(rect_at(i as f32 * 200.0));
        }
        let mut h = History::default();

        let edit = doc.erase_indices(vec![1, 3]).unwrap();
        h.record(edit);
        assert_eq!(doc.items.len(), 3);

        h.undo(&mut doc);
        assert_eq!(doc.items.len(), 5);
        for i in 0..5 {
            assert_eq!(
                doc.items[i].bounds().min.x,
                i as f32 * 200.0 - 1.5,
                "第 {i} 个图元位置不对，说明插回顺序有误"
            );
        }
    }

    #[test]
    fn clear_roundtrip_restores_everything() {
        let mut doc = Document::new();
        for i in 0..3 {
            doc.items.push(rect_at(i as f32 * 100.0));
        }
        let mut h = History::default();

        let edit = doc.clear_annotations().unwrap();
        h.record(edit);
        assert!(doc.items.is_empty());

        h.undo(&mut doc);
        assert_eq!(doc.items.len(), 3);

        h.redo(&mut doc);
        assert!(doc.items.is_empty());
    }

    #[test]
    fn translate_roundtrip() {
        let mut doc = Document::new();
        doc.items.push(rect_at(0.0));
        let mut h = History::default();

        let edit = doc.translate_items(&[0], 30.0, 40.0).unwrap();
        h.record(edit);
        assert_eq!(doc.items[0].bounds().min.x, 30.0 - 1.5);

        h.undo(&mut doc);
        assert_eq!(doc.items[0].bounds().min.x, -1.5);
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut doc = Document::new();
        let mut h = History::default();

        record_add(&mut h, &mut doc, 0.0);
        h.undo(&mut doc);
        assert!(h.can_redo());

        // 撤销之后又画了一笔 → 重做分支作废
        record_add(&mut h, &mut doc, 500.0);
        assert!(!h.can_redo(), "新操作必须清空重做栈");
        assert_eq!(doc.items.len(), 1);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut doc = Document::new();
        let mut h = History::new(3);

        for i in 0..5 {
            record_add(&mut h, &mut doc, i as f32);
        }
        assert_eq!(h.undo_len(), 3, "超出容量应淘汰最旧记录");

        // 只能撤销 3 次，剩下 2 个图元撤不掉
        for _ in 0..3 {
            h.undo(&mut doc);
        }
        assert!(!h.can_undo());
        assert_eq!(doc.items.len(), 2, "被淘汰的历史无法再撤销");
    }

    #[test]
    fn capacity_zero_is_clamped_to_one() {
        let h = History::new(0);
        assert_eq!(h.capacity(), 1);
    }

    #[test]
    fn peek_labels() {
        let mut doc = Document::new();
        let mut h = History::default();
        assert!(h.peek_undo_label().is_none());

        record_add(&mut h, &mut doc, 0.0);
        assert_eq!(h.peek_undo_label(), Some("添加标注"));

        h.undo(&mut doc);
        assert_eq!(h.peek_redo_label(), Some("添加标注"));
    }

    #[test]
    fn clear_empties_both_stacks() {
        let mut doc = Document::new();
        let mut h = History::default();
        record_add(&mut h, &mut doc, 0.0);
        h.undo(&mut doc);
        h.clear();
        assert!(!h.can_undo() && !h.can_redo());
    }

    #[test]
    fn undo_on_empty_stack_is_none() {
        let mut doc = Document::new();
        let mut h = History::default();
        assert!(h.undo(&mut doc).is_none());
        assert!(h.redo(&mut doc).is_none());
    }

    #[test]
    fn many_edits_roundtrip_in_any_order() {
        // 混合各种操作，连续撤销到底再全部重做，验证状态完全一致
        let mut doc = Document::new();
        let mut h = History::default();

        record_add(&mut h, &mut doc, 0.0);
        record_add(&mut h, &mut doc, 200.0);

        let path = Annotation::Path {
            points: vec![Pos::new(0.0, 0.0), Pos::new(10.0, 10.0)],
            kind: PathKind::Pen,
            style: Style::default(),
        };
        doc.items.push(path.clone());
        h.record(Edit::Add { item: path.clone() });

        let edit = doc.translate_items(&[0], 5.0, 5.0).unwrap();
        h.record(edit);

        let edit = doc.erase_indices(vec![1]).unwrap();
        h.record(edit);

        let snapshot_after = doc.items.clone();

        // 一路撤销到底
        let mut guard = 0;
        while h.can_undo() {
            h.undo(&mut doc);
            guard += 1;
            assert!(guard < 100, "撤销无法收敛");
        }
        assert!(doc.items.is_empty(), "撤销到底应回到空状态");

        // 再全部重做
        while h.can_redo() {
            h.redo(&mut doc);
        }
        assert_eq!(doc.items, snapshot_after, "重做后应精确还原");

        // 顺带验证 arrow 变体也能正确入栈
        doc.items.push(Annotation::Arrow {
            from: Pos::ZERO,
            to: Pos::new(100.0, 100.0),
            kind: ArrowKind::Elbow,
            style: Style::default(),
        });
        h.record(Edit::Add {
            item: doc.items.last().unwrap().clone(),
        });
        h.undo(&mut doc);
        assert_eq!(doc.items.len(), snapshot_after.len());
    }
}
