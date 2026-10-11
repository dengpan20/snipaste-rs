# 中文输入法（IME）方案详细设计

> 对应 `crates/app`（输入路由）+ `crates/render`（预编辑串渲染）
> 上游文档：`docs/architecture.md` §4.6
> 版本：v1.0 · 2026-10-11
> **状态：待验证。** 本方案的多数结论来自 API 与 changelog 分析，必须通过 §8 的实测清单确认。

---

## 1. 为什么这是本项目最大的技术风险

### 1.1 什么是"预编辑"

用户想输入"你好"，实际操作是敲 `n` `i` `h` `a` `o`。在这个过程中：

```
用户按键:  n  →  ni → nih → niha → nihao
屏幕上应显示: n  →  ni → nih → niha → nihao     ← 带下划线的"预编辑串"
                                          ↓ 空格/回车选词
实际插入文本:  你好                                ← "上屏"
```

**关键点**：`nihao` 这串字符并没有进入我们的文本框，它处于 IME 引擎的内部状态里，
应用需要通过操作系统的事件通道**拿到它并自己画出来**。

如果应用不做这件事，用户敲拼音时会看到输入框毫无反应，直到选词才突然蹦出两个字。
这在中文用户看来就是"输入法坏了"。

### 1.2 为什么原生 Rust 应用天然没有这个能力

浏览器和 Electron 之所以"天然支持"，是因为 Chromium 把 IME 集成整套做完了。
原生窗口框架（winit）只提供**最底层的原始事件**，预编辑串的排布、光标、下划线、
候选框定位，全部要应用自己接。

### 1.3 我们的有利条件

好消息是**我们不需要从零实现**。egui 从 0.35 起把 IME 作为重点改进项：

| egui 版本 | 相关改动 |
|---|---|
| 0.35 | "Better IME composition"、"Improve IME handling, add public method `owns_ime_events` on `Memory`" |
| 0.35+ | "Implement proper visuals for IME composition" —— 预编辑串用下划线标示，并在合成期间正确显示光标 |
| 0.36 | 延续改进，移动端键盘相关修复 |

winit 侧同样在维护：

| winit 版本 | 相关改动 |
|---|---|
| 0.30.11 | 修复 macOS 拼音输入时 `set_marked_text` 越界导致的**崩溃** |
| 0.30.13 | 修复 Windows `WM_IME_SETCONTEXT` 的 IME UI 标志位掩码 |

**结论：`egui::TextEdit` + `egui-winit` 已经有相当完整的 IME 支持。**
我们的工作是：正确的**接入**、正确的**光标位置上报**、以及准备好**兜底路径**。

---

## 2. 技术前提：各平台的 IME 通道

### 2.1 Windows

| 层 | 机制 |
|---|---|
| 传统 | IMM32：`WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION` / `WM_IME_ENDCOMPOSITION`，配合 `ImmGetCompositionStringW(GCS_COMPSTR)` |
| 现代 | TSF（Text Services Framework）：IME 通过 `ITextStoreACP` 与应用交换文本与光标位置 |
| 候选框定位 | `ImmSetCompositionWindow(CFS_POINT)` / `ImmSetCandidateWindow`；TSF 下由 IME 自行根据 caret 决定 |

Windows 11 的微软拼音是 TSF 实现，第三方输入法（搜狗、微信输入法）多为 TSF + 自绘候选框。

### 2.2 macOS

`NSTextInputClient` 协议，核心方法：

- `setMarkedText(_:selectedRange:replacementRange:)` —— 预编辑串
- `insertText(_:replacementRange:)` —— 上屏
- `firstRect(forCharacterRange:actualRange:)` —— **告诉输入法候选框该画哪儿**

winit 实现了这个协议。最后一项如果有偏差，候选框就会飞到屏幕角落。

### 2.3 winit 提供的抽象

winit 把上述平台差异收敛成两个 `WindowEvent`：

```rust
WindowEvent::Ime(Ime::Enabled)                    // 输入法激活
WindowEvent::Ime(Ime::Preedit { text, cursor })   // 预编辑串 + 串内光标位置
WindowEvent::Ime(Ime::Commit(text))               // 上屏
WindowEvent::Ime(Ime::Disabled)                   // 输入法失活
```

以及两个必须由我们主动调用的窗口方法：

```rust
window.set_ime_allowed(true / false);
window.set_ime_cursor_area(position, size);   // ← 候选框定位的关键
```

> **`set_ime_cursor_area` 是 winit 0.30 的新 API**，取代了已废弃的 `set_ime_position`。
> 它接受位置与尺寸两个参数，正好用于把"文字光标的矩形"告诉 IME。
> 这是解决候选框错位问题的**正解**，而不是像旧方案那样只能调 `ImmSetCompositionWindow`。

---

## 3. 方案总体设计

```
用户点「文字」工具并在画布上点击
        │
        ▼
Document::editing_text = Some(idx)      ← core 层的状态
        │
        ▼
app 层进入「文字输入模式」：
  1. window.set_ime_allowed(true)       ← 允许 IME
  2. 在该位置创建一个 egui TextEdit，并 request_focus()
        │
        ▼
每帧（在 egui 的 update 闭包里）：
  3. TextEdit 报告自己的光标矩形（galley cursor rect）
  4. 把该矩形从「窗口逻辑坐标」换算成 VSS → 上报
        │
        ▼
handle_platform_output 阶段：
  5. window.set_ime_cursor_area(cursor_rect)   ← 候选框跟着光标走
        │
        ▼
Esc / 点击别处 / Ctrl+Enter 结束编辑：
  6. window.set_ime_allowed(false)
  7. 把 TextEdit 的内容提交为 Annotation::Text
  8. Document::editing_text = None
```

### 3.1 关键设计决策

| 决策 | 选择 | 理由 |
|---|---|---|
| 文本编辑用什么承载 | **`egui::TextEdit`** | 不要自研。预编辑渲染、光标、选区、撤销、复制粘贴、双击选词，自研成本是按月计的 |
| 文字模型怎么存 | 存最终字符串 + 位置 + 样式 | 不存编辑历史；文字图元的编辑历史由全局 `History` 承担 |
| 何时开启 IME | 仅进入文字编辑模式时 | 平时关掉，避免 IME 抢走快捷键（例如拼音输入法会吞掉字母键） |
| 预编辑串谁来画 | egui | 交给 `TextEdit`，它已正确实现下划线与光标 |
| 候选框位置 | `set_ime_cursor_area` | 见 §4 |

### 3.2 ⚠️ 必须关掉 IME 的场景

这是极易被忽略、但影响很大的一个点：**全局快捷键是字母键时，IME 会吞掉它们。**

例如用户把截图快捷键设成 `Alt+A`，在文字编辑模式之外按 `Alt+A` 是没问题的
（IME 只处理无修饰键的字母）；但如果用户按 `A` 切换箭头工具，而 IME 处于激活状态，
这个 `A` 会被 IME 吃掉变成拼音输入。

因此规则是：

- **未进入文字编辑模式时，`set_ime_allowed(false)`**
- 进入文字编辑模式时开启，退出时立即关闭

这一条应当写进 `app` 层的状态机，而不是散落在各处。

---

## 4. 候选框定位 —— 已通过源码核实，风险大幅下降 ✅

### 4.1 问题背景：为什么它在"全屏遮罩"场景下会被放大

普通应用里，输入框通常离窗口左上角不远，即使 IME 用"窗口原点"当候选框锚点，
偏差也就几十像素，可以忍。

但我们的遮罩窗口是**覆盖整块屏幕、原点在屏幕左上角**的。用户在屏幕正中间建一个
文字框开始输入，如果候选框跑到窗口原点，它会出现在**屏幕左上角**，离光标一屏之遥。

这不是"体验略差"，而是"完全没法用"。

### 4.2 源码核实结论（2026-10-11）

对 `egui-winit 0.36.2` 的源码逐行核对后确认：**这两件事它已经自动做完了**。

引用 `egui-winit-0.36.2/src/lib.rs`（`handle_platform_output`）：

| 行号 | 代码 | 含义 |
|---|---|---|
| 1146-1154 | `let allow_ime = ime.is_some(); if is_toggling_ime { window.set_ime_allowed(allow_ime) }` | **IME 的开关由 egui 的 IME 输出自动驱动** |
| 1173-1185 | `let ime_rect_px = pixels_per_point * ime.rect; window.set_ime_cursor_area(PhysicalPosition{ x: ime_rect_px.min.x, .. }, PhysicalSize{ .. })` | **候选框定位自动完成，且已做逻辑→物理坐标换算** |
| 1164-1167 | `window.set_ime_purpose(to_winit_ime_purpose(ime.purpose))` | 输入目的（普通文本 / 密码 / 数字）自动同步 |
| 1156-1162 | `window.set_ime_allowed(false); window.set_ime_allowed(true)` | 需要打断合成时自动重启 IME |

同时确认 winit 0.30.13 确实提供了对应 API：
`Window::set_ime_cursor_area<P: Into<Position>, S: Into<Size>>`（`window.rs:1248`）
与 `Window::set_ime_allowed(bool)`（`window.rs:1283`）。

**结论：路径 A 成立，我方零额外工作。** 只需要 `TextEdit` 正常获得焦点，
egui 就会产出 IME 输出，egui-winit 就会把它转发给系统。**无需自己写任何 IME 定位代码。**

### 4.3 与"每屏一个遮罩窗口"架构的契合点（关键）

`pixels_per_point` 取的是**该窗口**的缩放倍率，不是全局值。

由于本方案采用「每显示器一个遮罩窗口」：

- 每个窗口的原点 = 该显示器的左上角
- 每个窗口的 `pixels_per_point` = 该显示器的 `scale_factor`

因此 `ime.rect × pixels_per_point` 得到的**正好就是该显示器的物理坐标**，
正是 `set_ime_cursor_area` 需要的值 —— **完全不需要 `VssToScreen` 介入**。

反过来说：如果当初选了"单个跨屏大窗口"的架构，这一步在多屏混合 DPI 下
必然算错（整个窗口只能有一个 `pixels_per_point`）。这是「每屏一个窗口」决策
带来的又一个额外收益，已补充进架构文档。

### 4.4 仍需实测的部分（源码看不出来的）

源码只能证明"管线通了"，不能保证"结果对"。以下四项必须实测：

| 待验证 | 为什么源码层面无法判断 |
|---|---|
| 第三方输入法（搜狗 / 微信）候选框是否跟随 | 它们可能自绘候选框，不走系统通道 |
| macOS 的候选框定位 | `firstRectForCharacterRange` 由 winit 自己实现，返回值是否正确需实测 |
| 屏幕边缘的自动翻转 | 由各 IME 自行决定，不受应用控制 |
| 混合 DPI 下第二块屏 | `pixels_per_point` 是否随窗口正确刷新 |

### 4.5 降级路径（仅在实测失败时启用）

**路径 B —— 已由 egui-winit 覆盖，无需自己实现。**

若将来发现 egui 提供的 `ime.rect` 取值有误，可用 egui 的
`ViewportCommand::IMERect(rect)` 手动覆盖（egui-winit lib.rs:1883-1891 会把它
转成 `set_ime_cursor_area`）。这是一条比"自己调 winit API"更省事的通道，
因为换算仍由 egui-winit 负责。

**路径 C：Win32 兜底（`ImmSetCompositionWindow`）**

若对某些第三方输入法无效，补一层 IMM32 调用。需要从
`window.window_handle()` 解出 `HWND`，并引入 `windows-sys` 的 IMM 模块。
只对 IMM32 系输入法有效，TSF 系（微软拼音）无效。

**路径 D：系统原生输入框（终极兜底）**

弹出一个独立的小型原生输入窗口，内含系统原生文本控件。用户在那里输入
（IME 行为 100% 正确），确认后把文本贴回画布。

代价是交互多一步、视觉上多一个弹窗，但**它保证可用**。必须作为设计的
一部分提前想好，而不是等出问题再说。

### 4.6 决策树（已按源码核实结果更新）

```
默认状态：采用「路径 A」—— egui-winit 自动处理，我方零代码
  │
  └─ E3 实测（§8）发现候选框位置错误
       │
       ├─ egui 的 ime.rect 取值有误
       │     └─► 用 ViewportCommand::IMERect 覆盖（路径 B'，仍复用 egui-winit 的换算）
       │
       ├─ 仅微软拼音异常
       │     └─► 加路径 C（IMM32：ImmSetCompositionWindow）
       │
       └─ 普遍异常（含第三方输入法）
             └─► 路径 D（系统原生输入框），并把路径 C 作为后续优化
```

> **重要**：路径 B（自己调用 winit 的 `set_ime_cursor_area`）**已从方案中移除**。
> 源码核实表明 egui-winit 已经做了这件事，并且额外处理了坐标换算、IME 开关的
> 防抖、合成打断等细节。重复实现只会引入冲突。

---

## 5. 代码骨架

### 5.1 app 层的输入路由

```rust
/// 文字编辑模式的进入 / 退出。集中在一处，避免 set_ime_allowed 的调用散落。
impl App {
    fn begin_text_edit(&mut self, at: Pos) {
        let len = self.doc.items.len();
        self.doc.items.push(Annotation::Text {
            rect: Rect::from_points(at, Pos::new(at.x + 200.0, at.y + 28.0)),
            content: String::new(),
            style: self.doc.style,
        });
        self.doc.editing_text = Some(len);
        self.doc.set_tool(Tool::Text);

        // 打开 IME，并让文字框获得键盘焦点
        if let Some(w) = &self.overlay_window {
            w.set_ime_allowed(true);
        }
        self.text_focus_requested = true;
    }

    fn end_text_edit(&mut self, commit: bool) {
        let Some(idx) = self.doc.editing_text.take() else { return };

        // 关键顺序：先关 IME，再处理内容。
        // 反过来会在关闭瞬间收到一个 Commit 事件，把用户"取消"的内容又插回来。
        if let Some(w) = &self.overlay_window {
            w.set_ime_allowed(false);
        }

        if !commit {
            // 取消：若内容为空则整个删掉，否则保留已输入的文本
            if self.doc.items.get(idx).is_some_and(|a| {
                matches!(a, Annotation::Text { content, .. } if content.trim().is_empty())
            }) {
                self.doc.erase_indices(vec![idx]);
            }
        } else {
            // 提交后若是空文本也要删掉，避免留下看不见的空图元
            if self.doc.items.get(idx).is_some_and(|a| {
                matches!(a, Annotation::Text { content, .. } if content.trim().is_empty())
            }) {
                self.doc.erase_indices(vec![idx]);
            }
        }
    }
}
```

### 5.2 每帧同步光标矩形

```rust
/// 在 egui 的 update 闭包里，TextEdit 绘制完成后调用。
fn sync_ime_cursor(ui: &egui::Ui, response: &egui::Response, ctx: &egui::Context) {
    // egui 会在 TextEdit 内部记录光标矩形；拿到它并暂存，
    // 等 handle_platform_output 阶段再喂给 winit。
    if let Some(state) = ctx.memory(|m| m.text_edit_state(response.id)) {
        let rect = state.cursor_rect;   // 逻辑坐标
        IME_CURSOR.with(|c| *c.borrow_mut() = Some(rect));
    }
}
```

> 具体的 `TextEdit` 光标矩形的获取 API 名称需要在实现时对照 egui 0.36 的实际
> 源码确认（可能在 `TextEditState` 或 `egui::output::IMEOutput` 里）。
> 这里给出的是设计意图：**必须能拿到光标矩形**，否则路径 B 无从谈起。

---

## 6. 必须写进代码的三条硬规则

1. **进入 / 退出文字模式的 `set_ime_allowed` 必须成对出现**，且退出时**先关 IME 再处理内容**
   （顺序反了会把"取消编辑"时残留的 Commit 内容插回来）。
2. **候选框位置换算必须走 `VssToScreen`**，不允许手写 `* scale_factor` ——
   混合 DPI 下手写换算必然出错，且症状是"只在某一块屏上错位"，极难定位。
3. **`Tab` / 字母快捷键在文字编辑模式下必须让位给 `TextEdit`**。
   egui 的 `Memory::owns_ime_events()` 与 `wants_keyboard_input()` 是判断依据，
   不能无条件拦截键盘。

---

## 7. 兜底方案（路径 D）的具体设计

当所有 IME 定位方案都不可接受时启用。

```
用户点文字工具 → 不在画布上直接编辑，而是：
  1. 在点击位置创建一个"原生输入窗口"
     - 无边框、置顶、约 400×120 逻辑像素
     - 内含系统原生多行文本控件
     - 定位在点击处附近（自动翻转避免超出屏幕）
  2. 用户在其中输入（IME 行为完全由系统保证，100% 正确）
  3. 确认（Ctrl+Enter / 点击"确定"）→ 读取文本 → 创建 Annotation::Text → 关闭窗口
  4. Esc → 关闭窗口，不创建任何东西
```

**实现成本**：Windows 上需创建一个含 `EDIT` / `RichEdit` 控件的原生窗口；
macOS 上需一个含 `NSTextView` 的 `NSPanel`。这是一块独立的平台代码，
约 1~2 天工作量。

**是否要提前做**：不建议。先做 E3 实测，路径 A/B 有 70% 以上的概率能解决。
但**设计上要预留下这个口子**：文字工具的实现不应该把"直接内联编辑"写死，
应当能通过一个配置项切换到"外部输入"模式。

---

## 8. 实测验收清单（E3 实验）

这是 M2 阶段的准入门槛。**任何一项不通过，都要先解决再继续开发其他标注工具**，
因为文字是最常用的标注方式之一。

### 8.1 Windows

| # | 场景 | 期望 | 结果 |
|---|---|---|---|
| W1 | 微软拼音 —— 基础输入 | 预编辑串带下划线显示，候选框跟随光标 | ☐ |
| W2 | 微软拼音 —— 候选框位置 | 紧贴光标下方，不跑到屏幕角落 | ☐ |
| W3 | 搜狗输入法 | 同上 | ☐ |
| W4 | 微信输入法 | 同上 | ☐ |
| W5 | 在第二块屏（不同 DPI）上输入 | 候选框位置正确 | ☐ |
| W6 | 屏幕上边缘 / 下边缘建文字框 | 候选框自动翻转，不超出屏幕 | ☐ |
| W7 | 输入过程中按 `Esc` | 取消预编辑，不关闭截图 | ☐ |
| W8 | 输入过程中切换工具（点工具栏） | 内容正确提交，IME 被关闭 | ☐ |
| W9 | 输入中随机敲 `Tab` | 不触发工具切换 | ☐ |
| W10 | 多行文本 + 中文标点 | 标点正确，不出现方块 | ☐ |

### 8.2 macOS

| # | 场景 | 期望 | 结果 |
|---|---|---|---|
| M1 | 拼音输入 | 预编辑串显示，**不崩溃**（对应 winit 0.30.11 修复的那个 bug） | ☐ |
| M2 | 候选框位置 | 紧贴光标 | ☐ |
| M3 | 五笔 / 仓颉 | 正常 | ☐ |
| M4 | Retina 外接 + 内建屏混合 DPI | 候选框位置正确 | ☐ |
| M5 | 全屏应用（如全屏视频）上呼出并输入 | 正常 | ☐ |
| M6 | 连续快速输入 100 字 | 无丢字、无乱序 | ☐ |

### 8.3 通用

| # | 场景 | 期望 |
|---|---|---|
| C1 | 中英混输（`你好hello世界`） | 正确 |
| C2 | 输入后 `Ctrl+Z` 撤销 | 撤销的是整条文字图元，不是逐字撤销 |
| C3 | 文字图元双击重新编辑 | 能进入编辑态并保留原内容 |
| C4 | 文字内容含 emoji | 正常显示（egui 自带 emoji 字体） |

---

## 9. 字体：与 IME 无关但必须一起解决

即使 IME 完全正常，如果**中文字形缺失**，用户看到的仍然是方块（□□□）。

egui 内置字体只有 Latin 与少量 emoji，**不含任何 CJK 字形**。必须在启动时注册：

```rust
let mut fonts = egui::FontDefinitions::default();
fonts.font_data.insert(
    "cjk".to_owned(),
    Arc::new(egui::FontData::from_owned(cjk_bytes)),
);
// 插到 Proportional 家族的最前面，让中文优先用这个字体渲染
fonts.families
    .get_mut(&egui::FontFamily::Proportional)
    .expect("egui 默认家族必然存在")
    .insert(0, "cjk".to_owned());
ctx.set_fonts(fonts);
```

### 9.1 字体来源：两条路

**路线一：系统字体（当前采用，零体积代价）**

| 平台 | 候选（按优先级） |
|---|---|
| Windows | `C:\Windows\Fonts\Deng.ttf`（等线）→ `simhei.ttf`（黑体）→ `simkai.ttf`（楷体） |
| macOS | `/System/Library/Fonts/Supplemental/Songti.ttc` → `PingFang.ttc` |

优选 TTF 而非 TTC：`.ttc` 是字体集合，需要指定索引，`egui::FontData` 对该场景的
支持情况需实测。若必须用 TTC，需要确认能否指定 index。

**收益**：二进制体积不增加（当前目标 ≤30 MB）。

**代价**：
- 依赖系统已装该字体（Windows 中文版必然有；**英文版 Windows 可能没有**）
- 跨机器渲染外观不一致
- 分发时无法保证一致体验

**路线二：内嵌字体子集（发布前切换）**

用 Noto Sans SC 的 GB2312 子集（约 6763 常用字 + 标点 + Latin），
体积约 3 ~ 5 MB。SIL OFL 1.1 许可允许嵌入与再分发。

**决策：两段式。** 开发与验证阶段用路线一（立刻可用、零体积）；
M4 发布前切换到路线二，并保留"优先内嵌、缺失时回落系统字体"的加载顺序。

### 9.2 字体加载的失败处理

```rust
pub enum FontSource { Bundled, System(PathBuf), Fallback }
```

若三处都找不到，**不能 panic**。降级为 `Fallback`（用 egui 默认字体），
同时在日志里 WARN，并在设置页显示"当前环境缺少中文字体，中文可能显示为方块"。

一个截图工具因为找不到字体而启动失败，是最糟糕的失败方式。

---

## 10. 风险登记

| # | 风险 | 概率 | 影响 | 对策 |
|---|---|---|---|---|
| I1 | 候选框位置错误（路径 A 不成立） | **高** | 严重 | 路径 B（`set_ime_cursor_area`）→ 路径 C（IMM32）→ 路径 D |
| I2 | 第三方输入法与 egui 不兼容 | 中 | 中 | W3/W4 实测；必要时对特定输入法走路径 D |
| I3 | 混合 DPI 下候选框位置错 | 中 | 中 | 强制走 `VssToScreen`；W5/M4 实测 |
| I4 | 系统的 F1 与 IME 功能键冲突 | 低 | 低 | macOS 上已通过改键规避 |
| I5 | 英文版系统缺中文字体 | 中 | 中 | 路线二内嵌子集；且提供 WARN 而非崩溃 |
| I6 | 中文标点（全角）宽度计算异常 | 低 | 低 | egui 的 `Galley` 处理字体度量，应当正确；列入 C 组实测 |

---

## 11. 结论

**可以做，但要当作一个独立课题来做，不能当成"顺手接一下"的功能。**

- 主干路径（路径 A/B）依赖 egui 0.36 + winit 0.30.13 现有能力，**有较大概率开箱可用**
- 真正的风险集中在**候选框定位**，而 `set_ime_cursor_area` 是针对性解法
- 必须准备好路径 D，并保证它在设计上不被堵死

**行动项（按顺序）**：

1. 先搭好最小骨架，跑通 egui 渲染（Phase 1，正在进行）
2. 骨架能跑后**立刻**做 E3 实测 —— 一个 200 行的 demo 就能验证路径 A/B
3. 实测结果回写本文档的 §8 表格，并据此决定是否启用路径 C/D
