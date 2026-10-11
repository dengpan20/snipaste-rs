# Snipaste 复刻版 —— 架构设计与开发方案

> **当前实现以 [Snipaste 对齐方案](./snipaste-alignment.md) 为准。** 下文保留原始技术规划；功能状态、里程碑及性能目标不能作为当前版本已经实现的证明。

> 项目代号：`snipaste-rs`
> 目标平台：Windows 10/11（x64、ARM64）、macOS 12.3+（Intel、Apple Silicon）
> 技术路线：**纯 Rust 原生**（winit + egui 自绘），不依赖 WebView / Electron
> 文档版本：v1.0 · 2026-10-11

---

## 1. 项目目标与需求拆解

### 1.1 产品定位

一个常驻托盘的跨端截图工具。按下 `F1` 立即冻结全屏、进入框选与标注，完成后可复制、另存为或"钉"在桌面上。与 Snipaste 的差异点是：**不支持贴图历史库/取色器等附加功能，先把截图+标注这条主线做到极致。**

### 1.2 非功能需求（这是选型的决定性因素）

| 需求 | 量化目标 | 说明 |
|---|---|---|
| 占用内存小 | 空载 ≤ 25 MB RSS；截图峰值 ≤ 180 MB（双 4K） | 常驻托盘，任何时刻不作为负担 |
| 性能高 | `F1` 按下 → 遮罩出现 ≤ 150 ms（双 4K）；标注拖拽稳定 60 FPS | 捕获会话预热是达标关键 |
| 开机自启 | 不需要管理员权限；启动时不弹窗、不抢焦点 | 落入用户级自启（注册表 Run / LaunchAgent） |
| 跨端一致 | 一套业务代码，平台差异全部收敛在 `platform` + `capture` 两个 crate | 禁止在业务层写 `#[cfg(windows)]` |
| 体积 | 单文件 exe ≤ 30 MB（含中文字体子集） | 影响字体内嵌策略 |

### 1.3 功能清单 → 技术实现映射

| # | 功能 | 实现方案 | 难度 |
|---|---|---|---|
| 1 | 椭圆、矩形 | 矢量图元 `Shape::Rect` / `Shape::Ellipse`，Shift 约束为正形 | ★ |
| 2 | 箭头 | 起点→终点 + 头部三角，支持直线/折线/曲线三种 | ★★ |
| 3 | 画笔 | 自由曲线，点采样 + Douglas-Peucker 抽稀 + Catmull-Rom 平滑 | ★★ |
| 4 | 记号笔 | 同画笔，但笔尖加宽 + 半透明 + Multiply 混合（荧光笔叠加效果） | ★★ |
| 5 | 马赛克 | 区域裁剪后降采样再放大（nearest），GPU shader 实时预览 | ★★★ |
| 6 | 文字 | 复用 `egui::TextEdit` 承载输入与 IME，模型只存渲染结果 | ★★★★ |
| 7 | 橡皮擦 | 局部恢复冻结底图（连续笔刷/框选，可撤销；0.2.1 已替换原对象删除方案） | ★★ |
| 8 | 撤回一步 | `History::undo()`，只回放矢量操作 | ★ |
| 9 | 下一步（重做） | `History::redo()` | ★ |
| 10 | 取消截图 | `Esc` / 右键，销毁遮罩窗口并释放纹理 | ★ |
| 11 | 钉在桌面 | 每个贴图 = 一个无边框置顶窗口，支持拖动/缩放/透明度/双击关闭 | ★★★ |
| 12 | 另存为 | `rfd` 原生保存对话框 + `image` 编码（PNG/JPG/WebP/BMP） | ★ |
| 13 | 识别二维码（选区内） | `rqrr` 识别，worker 线程执行，降采样加速 | ★★ |
| — | *（建议补充）复制到剪贴板* | `arboard`，同时写入 PNG 与文件路径 | ★ |

> **关于橡皮擦的重要决策**：不做像素级擦除。像素级擦除需要为每次描边保留独立位图层，撤销栈会退化成位图快照，内存随操作次数线性膨胀，直接违背"内存小"的硬性要求。对象级擦除在截图标注场景中体验几乎无差别，且天然可撤销。

---

## 2. 技术选型

### 2.1 结论

```
UI / 渲染  : egui 0.36 + egui-wgpu 0.36 + wgpu 30      （immediate mode，自绘）
窗口 / 事件 : winit 0.30                                 （跨平台窗口 + 事件循环）
屏幕捕获   : windows-capture 2（Win）/ ScreenCaptureKit（macOS）
全局热键   : global-hotkey 0.8
托盘       : tray-icon 0.26
剪贴板     : arboard 3
图像编解码 : image 0.25
二维码     : rqrr 0.11
原生对话框 : rfd 0.15
```

### 2.2 为什么是"纯 Rust 原生"而不是 Tauri / Electron

| 方案 | 空载内存 | 冷启动 | 中文字体/IME | 结论 |
|---|---|---|---|---|
| Electron | 200+ MB（一个 Chromium） | 1-3 s | 天然支持 | ❌ 直接违背内存要求 |
| Tauri 2 | 60-100 MB（系统 WebView） | 0.3-1 s | 天然支持（但 Win 用 WebView2、mac 用 WKWebView，两端行为有差异） | ⚠️ 可用但不够"轻" |
| 纯 Rust 原生 | **20-30 MB** | **< 200 ms** | 需自己解决（本方案用 egui 解决） | ✅ 符合全部硬性要求 |

唯一代价是中文输入法（IME）要自己踩坑，第三章 4.6 节给出具体评估与对策。

### 2.3 渲染后端：为什么选 wgpu 而不是 softbuffer

| 后端 | 峰值内存 | 4K 双屏合成 | 马赛克预览 | 生态 |
|---|---|---|---|---|
| `softbuffer`（纯 CPU 光栅化） | 最低 | 每帧 CPU 合成，拖动时吃一个核 | 需 CPU 重算 | `egui_softbuffer` 为第三方非官方后端 |
| **`wgpu`（egui-wgpu 官方）** | 纹理常驻显存，RSS 影响小 | GPU 合成，几乎无 CPU 占用 | shader 一个 pass 搞定，零延迟 | ✅ 官方维护 |

截图工具的特性是：**画面静止、图元很少、但分辨率极高**。这正是 GPU 最擅长的负载。选择 wgpu。

> 降级路径：若某些老旧/虚拟化环境（无 Vulkan/DX12/Metal）不可用，保留 `softbuffer + 自研简单光栅化` 作为 fallback。MVP 阶段不做。

### 2.4 依赖版本清单（2026-10 核实）

| Crate | 版本 | MSRV | 用途 |
|---|---|---|---|
| `winit` | 0.30.13 | 1.70 | 窗口、事件循环、显示器枚举 |
| `egui` | 0.36.2 | 1.95 | UI 控件、文字、IME、图元三角化 |
| `egui-wgpu` | 0.36.x | 1.95 | wgpu 渲染后端 |
| `wgpu` | 30.0.1 | 1.87 | GPU 抽象层 |
| `global-hotkey` | 0.8.0 | 1.77 | 全局热键（Win/macOS/X11） |
| `tray-icon` | 0.26.1 | — | 系统托盘 + 右键菜单 |
| `rqrr` | 0.11.0 | — | QR 码识别（quirc 的 Rust 移植） |
| `image` | 0.25.x | — | PNG/JPG/WebP/BMP 编解码 |
| `arboard` | 3.x | — | 剪贴板（图像 + 文本） |
| `rfd` | 0.15.x | — | 原生保存/打开对话框 |
| `windows` | 0.6x | — | Win32 API（DPI、窗口样式、注册表） |
| `objc2` + `objc2-app-kit` | 0.3.x | — | macOS NSWindow 层级与行为 |
| `windows-capture` | 2.0.1 | — | Windows Graphics Capture（Win 侧捕获） |
| `screencapturekit-rs` 或手写 FFI | — | — | macOS ScreenCaptureKit |

> **工具链要求**：`rustup` 与 **Rust ≥ 1.95**（egui 0.36 的 MSRV）。
> 本机已安装 **Rust 1.99.0 / `stable-x86_64-pc-windows-gnu`**，构建环境搭建全过程见
> [`docs/build.md`](./build.md)。

> **⚠️ M0 阶段的实际结论修正（2026-10-11）**
>
> 1. **本机用 GNU ABI 而非 MSVC**。原因是没有 MSVC Build Tools，也没有管理员权限。
>    实际落地的是 `x86_64-pc-windows-gnu` + w64devkit 2.10.0（GCC 16.2.0），
>    踩坑记录见 `docs/build.md` §4。**这不影响跨平台目标**：`windows-gnu` 是一级目标，
>    且本项目依赖全部是纯 Rust 绑定，不含需要编译的 C/C++。
>    macOS 侧不受影响，届时在 mac 机器 / CI 上构建。
> 2. **`wgpu` 必须作为 app 层的直接依赖**。初稿曾打算"绕道 `egui_wgpu::wgpu` 复用
>    同一份类型"，实际不可行 —— app 层要自建 `Surface`、配 `SurfaceConfiguration`、
>    编码 render pass，这些 API 都由 wgpu 提供。已加入 workspace 依赖：`wgpu = "30"`。
> 3. **`global-hotkey` 也需要显式声明**。初稿的依赖表漏了它。
> 4. **依赖版本已按实际 `Cargo.lock` 锁定**，见下方"实际锁定版本"。
>
> **实际锁定版本（`Cargo.lock`，M0 验证）**
>
> ```text
> winit            0.30.13      egui             0.36.2
> egui-wgpu        0.36.2       egui-winit       0.36.2
> wgpu             30.0.1       global-hotkey    0.8.0
> tray-icon        0.26.1       pollster         0.4.x
> ```
>
> 完整清单与校验和见仓库根目录 `Cargo.lock`（已入库，保证可复现构建）。

### 2.5 平台屏幕捕获 API 选型

**Windows**

| API | 性能 | 能捕获 DX 全屏应用 | 备注 |
|---|---|---|---|
| GDI `BitBlt` | 慢（30-80 ms/屏） | ❌ 拿不到独占全屏 | 兜底 |
| DXGI Desktop Duplication | 最快 | ✅ | 多显卡/混合适配器场景复杂，需自管 D3D11 设备 |
| **Windows Graphics Capture (WGC)** | 快（10-25 ms/屏） | ✅ | Win10 1903+，官方推荐，可捕获单窗口 |

选 **WGC**。关键优化：WGC 首次建立会话有 50-200 ms 的固定开销，因此**进程启动后预热一个常驻会话**，`F1` 触发时直接取帧，这是达成"150 ms 内出遮罩"的核心手段。

**macOS**

| API | 系统要求 | 备注 |
|---|---|---|
| `CGDisplayCreateImage` | 10.15+ | 已废弃，但兼容性最好，作为兜底 |
| **ScreenCaptureKit**（`SCScreenshotManager` / `SCStream` 取单帧） | 12.3+ / 14+ | 官方推荐，性能与色彩管理最好 |

选 **ScreenCaptureKit**，`CGDisplayCreateImage` 作为 12.3 以下兜底（若决定只支持 12.3+ 可完全去掉）。

---

## 3. 总体架构

### 3.1 分层结构

```
┌─────────────────────────────────────────────────────────────┐
│  app 层 —— 进程生命周期、事件循环、托盘、状态机、配置         │
│  （唯一持有 winit EventLoop 的地方；所有窗口都在这一层创建）  │
└───────┬─────────────────────────────────┬───────────────────┘
        │                                 │
┌───────▼────────────┐          ┌─────────▼───────────────────┐
│  ui / render 层     │          │  core 层（纯逻辑，无 IO）    │
│  egui 控件、工具栏  │◄────────►│  Document / Annotation      │
│  遮罩绘制、图元渲染 │          │  History / 坐标变换 / 命中   │
│  贴图窗口渲染       │          │  完全可单元测试，不依赖 OS   │
└────────────────────┘          └─────────────────────────────┘
        │                                 │
┌───────▼─────────────────────────────────▼───────────────────┐
│  service 层 —— 有副作用、跨线程                                │
│  capture（捕获会话池） · qr（识别 worker） · export（编码/剪贴板）│
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  platform 层 —— 唯一的平台差异收敛点                           │
│  win:  WGC / RegisterHotKey / Run 注册表 / PerMonitorV2 DPI  │
│  mac:  ScreenCaptureKit / Carbon HotKey / LaunchAgent / TCC  │
└─────────────────────────────────────────────────────────────┘
```

**铁律**：`core` 层不允许出现 `#[cfg(target_os)]`，不允许触碰文件系统和窗口。所有平台差异必须在 `platform` 层以 trait 形式抽象出来。

### 3.2 线程模型

```
主线程（必须）
├── winit EventLoop::run_app()            ← macOS 强制要求主线程
├── 遮罩窗口 / 贴图窗口的事件与渲染
└── 通过 EventLoopProxy 接收其他线程的消息

捕获线程（1 个，常驻）
├── 持有 WGC / SCK 会话，处于"预热"状态
└── 收到 CaptureRequest → 逐屏取帧 → 通过 channel 回传

识别线程池（rayon，2-4 线程，懒启动）
└── 二维码识别、PNG 编码（大图编码 200-500 ms，绝不能阻塞主线程）

编码/导出线程（1 个）
└── 保存文件、写剪贴板
```

线程间通信统一用 `crossbeam-channel`（或 `std::sync::mpsc`），事件回投主线程统一走 `EventLoopProxy<AppEvent>`。

```rust
enum AppEvent {
    HotkeyTriggered(HotKeyId),
    CaptureReady { shots: Vec<MonitorShot> },
    CaptureFailed(CaptureError),
    QrDecoded { region: Rect, payload: QrPayload },
    ExportDone { path: Option<PathBuf>, err: Option<String> },
    TrayMenu(MenuId),
}
```

### 3.3 workspace 划分

```toml
[workspace]
members = [
    "crates/app",       # 主二进制：事件循环、托盘、状态机、配置
    "crates/core",      # 领域模型：Document/Annotation/History/几何
    "crates/render",    # egui 主题、图元三角化、马赛克 shader、字体
    "crates/capture",   # 捕获抽象 + 会话池 + 帧队列
    "crates/platform",  # 热键/自启/剪贴板/DPI/窗口扩展/权限
]
```

好处：`core` 和 `render` 可以脱离操作系统做单元测试和离线渲染基准测试；平台实现编译期隔离（Windows 构建不会链接任何 objc 代码）。

---

## 4. 核心模块设计

### 4.1 屏幕捕获（capture）

**核心抽象**

```rust
pub struct MonitorShot {
    /// 显示器在物理像素虚拟桌面中的位置（原点为虚拟桌面左上角）
    pub origin: PhysicalPos,
    /// 该显示器的物理像素尺寸
    pub size: PhysicalSize,
    /// scale_factor（Retina = 2.0）
    pub scale_factor: f64,
    /// BGRA8 帧数据，Arc 共享，避免拷贝
    pub frame: Arc<FrameBuffer>,
}
```

**关键设计点**

1. **并行取多屏**：多显示器同时发起捕获（`std::thread::scope`），把 N 屏串行时间压成 1 屏时间。
2. **零拷贝**：WGC/SCK 返回的帧直接封装进 `Arc<FrameBuffer>`，向上传递时只克隆 `Arc`（原子计数），不复制像素。
3. **会话预热**：进程启动即创建捕获会话并保持空闲态，`F1` 时直接取帧。这是延迟指标的关键。
4. **画布坐标系**（最关键的设计）：定义 **VSS（Virtual Screen Space）**，以**物理像素**为单位、虚拟桌面左上角为原点，把多屏拼成一张逻辑大画布。

```
VSS 示例（双屏，左屏 1920×1080@1.0，右屏 3840×2160@2.0）

  VSS 原点
   (0,0)
     ┌───────────────┬──────────────────────────────┐
     │  monitor 0    │  monitor 1                   │
     │  1920×1080    │  3840×2160                   │
     │  scale = 1.0  │  scale = 2.0                 │
     └───────────────┴──────────────────────────────┘
                                                  VSS 右下角 (5760, 2160)
```

所有标注图元、选区、鼠标坐标统一在 VSS（物理像素）中表达；仅在**绘制到某个具体显示器的 egui 画布**时，才通过 `vss → 该屏物理 → 该屏逻辑点` 的变换矩阵转换。这样彻底规避混合 DPI 下的坐标错乱。

### 4.2 遮罩窗口（overlay）

**采用"每显示器一个窗口"而非"单个跨屏大窗口"。** 理由：混合 DPI 下，跨屏大窗口会被系统按某一个 scale 统一缩放，导致另一块屏的图像模糊/错位，且 hit-test 坐标换算极其痛苦。每屏一个窗口，每个窗口只画自己那一块，物理像素 1:1 对应，图像绝不模糊。

```
F1 按下
  ├─ 捕获全部显示器 → Vec<MonitorShot>   （并行，预热会话 → 10-25ms）
  ├─ 为每个显示器创建 winit 窗口：
  │    ├─ with_fullscreen(Some(Borderless))  或 显式设置 position + inner_size
  │    ├─ with_decorations(false)
  │    ├─ with_window_level(AlwaysOnTop)
  │    ├─ with_resizable(false)
  │    └─ with_visible(false)   ← 先隐藏，全部就绪后统一 show，避免逐屏闪现
  ├─ 全部窗口就绪 → 一次性 show 所有窗口（视觉上同时出现）
  └─ 创建 egui 上下文，把 frame 上传为 wgpu 纹理
```

**Windows 特有处理**
- 必须设置 **Per-Monitor DPI Awareness V2**，否则跨屏坐标会被系统偷偷缩放。通过 exe manifest 或运行时 `SetProcessDpiAwarenessContext`（winit 已处理大部分，需实测确认）。
- 遮罩窗口需要 `WS_EX_TOOLWINDOW`（不出现在 Alt+Tab）和抑制激活时的任务栏闪烁。

**macOS 特有处理**
- `NSWindow.level = NSScreenSaverWindowLevel`（高于普通置顶窗口），否则在全屏应用或菜单栏区域会被遮挡。
- `collectionBehavior` 必须加 `NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary`，否则用户切到别的 Space 时遮罩不显示。
- 窗口需 `acceptsFirstMouse = true`，否则第一次点击被吞掉。

**取消路径必须干净**：`Esc`/右键 → 销毁所有遮罩窗口 → 释放 wgpu 纹理 → 回到常驻态。所有纹理句柄在窗口销毁时同步 drop，避免显存泄漏（多次截图后显存不释放是这类工具的经典 bug）。

### 4.3 场景模型（core）

**Document = 一张待导出的画布**

```rust
pub struct Document {
    /// 选区在 VSS 中的矩形（物理像素）
    pub selection: Option<Selection>,
    /// 已提交的标注图元，按 z 序
    pub items: Vec<Annotation>,
    /// 正在绘制中的临时图元（未提交，不参与撤销）
    pub draft: Option<Annotation>,
    /// 当前工具与样式
    pub tool: Tool,
    pub style: Style,
}

pub struct Style {
    pub stroke: Color,
    pub stroke_width: f32,     // 物理像素
    pub fill: Option<Color>,
    pub font_size: f32,
    pub mosaic_block: u32,     // 马赛克块大小
    pub mosaic_mode: MosaicMode, // Pixelate | Blur
}

pub enum Annotation {
    Rect     { rect: Rect, style: Style },
    Ellipse  { rect: Rect, style: Style },
    Arrow    { from: Pos, to: Pos, kind: ArrowKind, style: Style },
    Path     { points: Vec<Pos>, kind: PathKind /* Pen | Marker */, style: Style },
    Mosaic   { region: MosaicRegion, style: Style },
    Text     { rect: Rect, content: String, style: Style },
    Step     { center: Pos, index: u32, style: Style },   // 序号标记（可扩展）
}
```

**关键决策：`Annotation` 只存矢量参数，绝不存位图。** 一个带有 500 个采样点的画笔描边 ≈ 4 KB；即使 100 个图元也只占几百 KB。这让撤销栈可以无上限保留。

### 4.4 撤销 / 重做（History）

```rust
pub enum Edit {
    Add(Annotation),           // 新增图元
    Remove(usize),             // 删除图元（橡皮擦）
    Move { id: usize, dx: f32, dy: f32 },   // 拖动调整位置
    Clear,                     // 清空全部
}

pub struct History {
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,     // 任何新操作都必须清空 redo_stack
}
```

- **不做快照式撤销**（每个状态存一份位图）—— 内存会爆炸。只记录操作。
- `Ctrl+Z` / `Ctrl+Shift+Z`（macOS 上 `Cmd+Z` / `Cmd+Shift+Z`）。
- 上限设为 **50 步**，超出丢弃最早的操作（并非为了内存——操作本身很小——而是防止极端情况下的语义混乱）。
- 每次 `undo` 后立即 `request_redraw`，`redraw` 时按 `items` 顺序重放，不做增量 diff。

### 4.5 工具栏与交互

**工具栏跟随选区**：当选区下方空间足够时，工具栏出现在选区下方外缘；不足时自动翻转到选区上方/内部。采用选区坐标系定位，不依赖屏幕绝对坐标。

**尺寸提示**：选区右上角显示 `宽 × 高`（物理像素），拖动时实时更新；若启用了"逻辑像素显示"，按 `scale_factor` 换算。

**交互状态机**

```
                 F1
  Idle ─────────────────► AwaitingSelection
   ▲                            │
   │ Esc/右键                   │ 鼠标按下并拖动
   │                            ▼
   │                      Selecting ◄──── 拖动中调整八向控制点
   │                            │
   │                            │ 松开
   │                            ▼
   │                      Annotating ──► DraggingItem（拖动已有图元）
   │                        │     ▲            │
   │                        │     └────────────┘
   │                        │
   │                        ├─ 绘制中：Drafting（松开鼠标 → 提交到 items）
   │                        ├─ 文字：EditingText（获焦的 egui TextEdit）
   │                        └─ 导出：
   │                             ├─ Ctrl+C / Enter / 双击  → 复制剪贴板
   │                             ├─ Ctrl+S                → 另存为
   │                             └─ Ctrl+T / 工具栏按钮    → 钉在桌面
   │
   └──────────────────────────────────────────────────────────
```

**快捷键表**

| 键 | 行为 |
|---|---|
| `F1` | 全局呼出截图（可配置） |
| `Esc` | 取消截图 / 取消当前绘制 |
| 右键 | 取消截图（与 Snipaste 一致） |
| `Tab` | 切换工具（循环） |
| `Ctrl+Z` / `Ctrl+Shift+Z` | 撤销 / 重做 |
| `Ctrl+C` / `Enter` | 复制到剪贴板并退出 |
| `Ctrl+S` | 另存为 |
| `Ctrl+T` | 钉在桌面 |
| `Ctrl+A` | 全屏选区 |
| 方向键 / `Shift`+方向键 | 选区微调 1px / 10px |
| `Ctrl` 拖动 | 复制一个图元 |
| `Shift` 拖动 | 约束为正方形/正圆/水平垂直箭头 |
| 滚轮 | 调整当前笔宽（或选中图元的缩放） |
| `[` / `]` | 笔宽减 / 增 |

### 4.6 文字标注与中文输入法（**本项目最大的技术风险**）

**风险描述**：中文/日文/韩文输入法（IME）需要"预编辑（composition）"支持——用户敲拼音时，窗口需要显示下划线拼音串和候选词，而不是直接上屏。原生 winit 应用不会自动获得这个能力，必须自己转发 IME 事件并渲染候选词窗口。

**好消息**：egui 在 0.35 起对 IME 做了明确改进（"Better IME composition"、"Improved IME handling"、`owns_ime_events`），winit 0.30.11 也修复了 macOS 拼音输入 `set_marked_text` 越界的崩溃。egui-winit 已经能把 `WindowEvent::Ime` 转成 `egui::Event::Ime`，`egui::TextEdit` 原生支持预编辑串渲染。

**方案**：

```
文字工具激活 → 在点击位置创建一个 egui TextEdit（多行）
  ├─ 该 TextEdit 取得焦点，获得完整的 IME 事件流
  ├─ egui 负责渲染预编辑串 + 光标
  ├─ 系统候选词窗口由 OS 自己绘制（egui-winit 会给出正确的光标位置）
  └─ TextEdit 失焦（点击别处 / Esc / Ctrl+Enter）→ 提交为 Annotation::Text
```

**必须实测验证的清单**（建议作为 M2 阶段的验收门槛）：

- [ ] Windows 10/11：微软拼音，输入过程中预编辑串正确显示，候选框位置跟随光标
- [ ] Windows：搜狗/微信输入法（这类第三方输入法会有差异，必须实测）
- [ ] macOS：拼音、五笔，`set_marked_text` 不崩溃，候选框位置正确
- [ ] macOS：日文假名输入（若考虑国际化）
- [ ] 兜底策略：若某平台 IME 体验不可接受，提供"外部编辑器"降级路径——弹出系统原生输入框，输入完成后贴回画布

**另一个必须提前处理的坑：egui 默认字体不含中文字形。**

egui 内置字体只有 Latin + 少量 emoji，直接渲染中文会得到"豆腐块"（□□□）。必须在 `FontDefinitions` 中注册中文字体：

```rust
let mut fonts = egui::FontDefinitions::default();
fonts.font_data.insert(
    "cjk".into(),
    egui::FontData::from_static(include_bytes!("../../assets/fonts/NotoSansSC-Subset.ttf")).into(),
);
fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().push("cjk".into());
fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().push("cjk".into());
ctx.set_fonts(fonts);
```

- 用**思源黑体（Noto Sans SC）子集**：只保留 GB2312 常用字（约 6763 字）+ 标点 + Latin，`ttf` 体积约 3-5 MB。全量字体 10+ MB，不划算。
- 许可：Noto Sans SC 为 SIL OFL 1.1，允许嵌入和再分发（需保留版权声明）。

### 4.7 各标注工具的绘制细节

**矩形 / 椭圆**
- 拖拽生成外接矩形 `Rect{min, max}`；`Shift` 约束为正方形/正圆。
- 支持空心（描边）/ 实心（填充）/ 半透明填充三种模式，与 Snipaste 一致。
- 绘制后支持八向控制点调整尺寸、拖动整体平移。

**箭头**
- `ArrowKind::Straight | Elbow（折线）| Curve（曲线）`
- 头部：以终点为顶点，按方向角 ±150° 生成等腰三角形，边长 = `4 × stroke_width`（保证任何笔宽下比例协调）。
- 曲线箭头用二次贝塞尔，控制点取中点垂直偏移 `0.25 × 长度`。

**画笔**
- 采样：鼠标移动事件按 **物理像素距离 ≥ 1.5 px** 才记录一个点（避免高刷鼠标产生冗余点）。
- 抽稀：Douglas-Peucker，容差 0.5-1.0 px，可减少 60-80% 的点而不损观感。
- 平滑：抽稀后的点用 Catmull-Rom 转三次贝塞尔，交给 egui 的 path tessellator 画。
- 渲染：`PathKind::Pen` 用圆头 + 圆角连接；描边宽度按物理像素存储，跨屏一致。

**记号笔**
- 与画笔共用几何，差异全在样式：
  - 笔尖宽度默认 `3×` 画笔
  - 颜色 alpha ≈ 0.4
  - 混合模式用 **Multiply**（叠加处变深，模拟荧光笔）
  - 端点/连接用 **平头 + 斜接**（而非圆头），更接近马克笔笔触
- 三次重叠处不能变成不透明黑：Multiply 的 alpha 累积需要钳制，或者直接把同一笔迹当作一个整体 mesh 一次性绘制（推荐后者，天然无叠加问题）。

**马赛克**
- 区域形状支持矩形与自由画笔（用画笔轨迹生成 mask）。
- **GPU 实现**（推荐）：
  1. 把选区内的原图作为纹理 `TexSrc`
  2. 在片元着色器中按块坐标取整采样：`uv = floor(uv * size / block) * block / size`
  3. 模糊模式则做 3×3 或 5×5 盒式/高斯采样
  4. 用一个只覆盖该区域的 quad 绘制，混合模式为替换
- 块大小 `block ∈ [4, 40]`（物理像素），滚轮实时调整，GPU 预览零延迟。
- **CPU 降级实现**（MVP 可先用）：裁剪 → `image::imageops::resize` 缩小到 `1/block` → 再 `resize` 回原尺寸（Nearest 过滤）。1 MP 区域约 8-15 ms，一次性计算可接受。

**橡皮擦**
- 命中测试：从 z 序**从后往前**遍历 `items`，第一个命中（点到线距离 < `stroke_width/2 + 6`，或被填充区域包含）的图元被删除。
- 笔尖绘制为圆形光标 + 实时高亮"将被删除的图元"（描红/加光晕），给出明确的预反馈。
- 支持按键拖动连续擦除（一次拖动可删除多个图元），但**整段拖动只算一个 `Edit`**（合并为 `Edit::RemoveMany`），避免撤销要多按很多次。

**文字**
- 见 4.6。渲染时用 egui 的 `Galley` 布局，支持自动换行与多行。
- 提交后仍是可编辑对象：双击进入编辑。
- 字体大小支持 `8 / 12 / 16 / 24 / 32` 预设 + 自由输入。

### 4.8 二维码识别（qr）

```rust
// service/qr.rs  在 worker 线程执行
pub fn decode_region(frame: &FrameBuffer, region: Rect) -> Vec<QrPayload> {
    // 1. 裁剪 + 转灰度
    let gray = crop_to_luma8(frame, region);
    // 2. 若长边 > 1600，先降采样（识别精度几乎无损，速度提升 3-5×）
    let gray = maybe_downscale(gray, 1600);
    // 3. rqrr
    let mut img = rqrr::PreparedImage::prepare(gray);
    let grids = img.detect_grids();
    grids.iter().filter_map(|g| {
        let (meta, content) = g.decode().ok()?;
        Some(QrPayload { content, bounds: g.bounds, version: meta.version, ecc: meta.ecc_level })
    }).collect()
}
```

**交互设计（"选择后"）**
- 用户点击二维码工具 → 光标变十字 → 框选一个区域 → 松开即触发识别。
- 识别成功：在选区旁弹出结果卡片，提供「复制内容」「打开链接」（若内容是 URL）两个动作，并在二维码位置画出高亮框。
- 识别失败：卡片显示「未识别到二维码」，可重试或扩大选区。
- **增强建议**：加一个「全屏自动检测」模式，扫描整张冻结图，把所有二维码位置一次性框出来，点击任意一个查看内容。

**性能**：4K 全图识别约 150-300 ms，必须异步。UI 期间显示 loading 指示，不阻塞主线程。

**扩展**：`rqrr` 只支持 QR Code。若后续需要识别条形码（EAN-13、Code128 等），需替换为 `rxing`（ZXing 的 Rust 移植），体积更大但格式覆盖全。

### 4.9 钉在桌面（pin）

每个贴图是一个独立的 winit 窗口，由 `app` 层统一管理：

```rust
pub struct PinWindow {
    window: Window,
    texture: wgpu::Texture,      // 合成后的最终图像（含标注）
    image_size: PhysicalSize,
    scale: f32,                  // 用户缩放
    opacity: f32,                // 0.1 - 1.0
    anchored: bool,              // 贴边吸附
}
```

**跨平台窗口属性**

| 属性 | Windows | macOS |
|---|---|---|
| 置顶 | `SetWindowPos(HWND_TOPMOST)` 或 winit `AlwaysOnTop` | `NSWindow.level = NSFloatingWindowLevel` |
| 无边框 | `with_decorations(false)` | 同 |
| 不出现在任务栏 | `WS_EX_TOOLWINDOW` | `LSUIElement` / `NSWindowCollectionBehaviorTransient` |
| 跨 Space 可见 | — | `CanJoinAllSpaces` |
| 不抢焦点（可选） | `WS_EX_NOACTIVATE` | `NSWindowStyleMaskNonactivatingPanel` |
| 拖动 | 监听鼠标拖拽 → `window.set_outer_position()` | 同 |

**交互**
- 拖动：按住图像任意位置拖动窗口（自己实现，不用系统标题栏）
- 缩放：滚轮（等比）+ 角点拖动（自由）
- 透明度：`Ctrl + 滚轮`
- 双击：关闭
- 右键：菜单（复制 / 另存为 / 置顶切换 / 透明度 / 关闭）
- 贴边吸附：拖动结束时若距屏幕边缘 < 12 px，吸附并对齐
- **性能优化**：贴图窗口内容静态，只在尺寸/透明度变化时重绘（egui 场景下用 `request_redraw` 主动触发，其余时间不产生新帧），空载 CPU ≈ 0。

### 4.10 导出（export）

**另存为**
- 默认格式 PNG（无损），可选 JPG / WebP / BMP
- 默认目录：`~/Pictures/Snipaste` / `%USERPROFILE%\Pictures\Snipaste`（记忆上次目录）
- 默认文件名：`Snipaste_2026-10-11_17-49-34.png`（可自定义模板）
- JPG 质量默认 92，可配置
- 使用 `rfd` 原生保存对话框（macOS 上是 NSSavePanel，Windows 上是 IFileSaveDialog）
- 编码在 worker 线程执行（4K PNG 编码 200-500 ms）
- **可选增强**：保存后自动复制到剪贴板 / 自动打开所在文件夹 / 保存后贴图

**复制到剪贴板**
- Windows：`CF_DIBV5`（保留 alpha）+ `CF_HDROP`（文件路径）
- macOS：`NSPasteboard` 写 `public.png` + `public.file-url`
- 同时把 PNG 字节写入剪贴板，方便粘贴到不支持文件路径的软件

**导出图像合成流程**（关键，必须一次做对）

```
1. 按 selection 从原始 MonitorShot frame 裁剪出 base bitmap（物理像素）
   └─ 先不考虑标注，得到纯净底图
2. 若 selection 跨多个显示器 → 先把各屏裁剪块拼接成一张完整底图
3. 把 items 逐层绘制到底图上：
   ├─ 矢量图元：用与屏幕渲染完全相同的 tessellation 参数光栅化
   │            （推荐直接复用 wgpu 管线，渲染到离屏纹理后 readback）
   └─ 马赛克：按同样算法应用
4. readback 到 CPU → image::DynamicImage → 编码
```

> **一致性保证**：屏幕预览和最终导出必须走**同一套 tessellation 与 shader**，否则会出现"所见非所得"（最常见的表现是导出后描边粗细或马赛克块大小和预览不一样）。实现上建议：把导出也当作一次 GPU 渲染——渲染到离屏纹理，再把纹理 readback 出来编码。这样绝对一致。

---

## 5. 关键数据结构汇总

```rust
// ---- 基础几何（全部使用物理像素）----
pub struct Pos  { pub x: f32, pub y: f32 }          // VSS 坐标
pub struct Rect { pub min: Pos, pub max: Pos }

// ---- 显示器与捕获 ----
pub struct MonitorInfo { pub id: u32, pub vss_rect: Rect, pub scale: f64, pub name: String }
pub struct MonitorShot { pub info: MonitorInfo, pub frame: Arc<FrameBuffer> }
pub struct FrameBuffer { pub width: u32, pub height: u32, pub bgra: Vec<u8> }

// ---- 文档与标注 ----
pub struct Document { selection: Option<Rect>, items: Vec<Annotation>,
                      draft: Option<Annotation>, tool: Tool, style: Style }
pub enum Annotation { Rect{..}, Ellipse{..}, Arrow{..}, Path{..},
                      Mosaic{..}, Text{..}, Step{..} }

// ---- 历史 ----
pub enum Edit { Add(Annotation), Remove(usize), RemoveMany(Vec<usize>),
                Move{ id: usize, dx: f32, dy: f32 }, Clear }
pub struct History { undo: Vec<Edit>, redo: Vec<Edit>, cap: usize /* = 50 */ }

// ---- 应用级状态 ----
pub enum Mode {
    Idle,                                        // 常驻托盘
    Capturing,                                   // 遮罩存在，正在框选/标注
    Pin(Vec<PinWindow>),                         // 贴图窗口
    Settings,                                    // 设置窗口
}
```

---

## 6. 核心流程时序

### 6.1 `F1` → 遮罩出现（目标 < 150 ms）

```
用户按 F1
  │
  ├─ global-hotkey 触发 → GlobalHotKeyEvent → AppEvent::HotkeyTriggered
  │                                      （Windows: WM_HOTKEY，macOS: Carbon 回调）
  ├─ 主线程：立即检查 Mode 是否允许（Idle 才响应，避免重复触发）
  ├─ 主线程：向捕获线程投递 CaptureRequest
  │
  ├─ 捕获线程（已有预热会话）：
  │    ├─ 并行对所有显示器取帧（thread::scope）
  │    └─ 帧封装为 Arc<FrameBuffer> → AppEvent::CaptureReady
  │
  ├─ 主线程收到：为每个显示器创建窗口（不可见）
  │    ├─ 创建 wgpu surface（复用同一个 Device/Queue，只创建 Surface）
  │    ├─ 上传帧为纹理
  │    └─ 配置 egui Context 与输入状态
  │
  └─ 所有窗口就绪 → show() 全部 → 用户看到冻结的屏幕
```

**延迟预算拆解**（双 4K 目标 150 ms）

| 阶段 | 预算 | 优化手段 |
|---|---|---|
| 热键回调 → 主线程 | < 5 ms | 无锁 channel |
| 捕获取帧（2 屏并行） | 25-60 ms | WGC/SCK 会话预热；并行 |
| 帧上传 GPU | 10-25 ms | 复用 buffer，避免 PCIe 往返 |
| 创建 2 个 Surface + 首帧 | 30-60 ms | 启动时预热一个隐藏窗口的 wgpu 上下文（可选激进优化） |
| egui 首帧布局 | 5-15 ms | 遮罩场景下 `ctx.set_fonts` 已就绪，不做重布局 |

### 6.2 框选 → 标注 → 导出

```
鼠标按下（在某个显示器窗口内）
  ├─ 把该窗口的本地逻辑坐标 → 该屏物理坐标 → VSS 坐标
  └─ 记录 drag_start（VSS）

鼠标拖动
  ├─ 实时更新 selection（VSS）
  ├─ 若拖到另一块屏，跨屏窗口的绘制由各自的窗口负责各自部分
  │   （各窗口只画 VSS ∩ 本屏矩形 的裁剪结果）
  ├─ 每个窗口 request_redraw()（仅拖动期间）
  └─ 松开鼠标 → selection 固定，进入 Annotating

标注阶段
  ├─ 每次鼠标事件更新 draft / items
  ├─ 橡皮擦 / 撤销 → 修改 items + history
  └─ request_redraw → 重绘当前屏（只重绘鼠标所在屏，其他屏保持）

导出
  ├─ Ctrl+C  → 合成 → 写剪贴板 → 销毁遮罩 → 回到 Idle（可选显示 toast）
  ├─ Ctrl+S  → 合成 → rfd 对话框 → worker 编码保存
  └─ Ctrl+T  → 合成 → 创建 PinWindow → 销毁遮罩
```

---

## 7. 难点、风险与对策

| # | 风险 | 影响 | 对策 |
|---|---|---|---|
| R1 | **macOS 屏幕录制权限**（TCC） | 首次调用捕获返回黑屏 | `CGPreflightScreenCaptureAccess()` 预检；未授权则弹自定义引导页并打开「系统设置 → 隐私与安全性 → 屏幕录制」；**授权后通常需重启进程才生效**（macOS 15 部分场景可即时生效，必须实测）。README 必须先讲这一步 |
| R2 | macOS 上 `F1` 默认是「降低亮度」 | 用户按 F1 调亮度而非截图 | 默认热键仍设 F1，但明确提示用户：需在「系统设置 → 键盘」中关闭「将 F1、F2 等键用作标准功能键」，或在应用内改键为 `Ctrl+F1` |
| R3 | Windows `RegisterHotKey` 会吞掉 F1 | 前台程序的 F1 帮助失效 | 这是全局热键的固有行为（Snipaste 亦然）。必须提供热键自定义，并在设置页说明 F1 的副作用 |
| R4 | 中文 IME 预编辑 | 中文无法正常输入 | 见 4.6。egui 0.36 + winit 0.30.13 已有改善，但仍需逐平台实测；准备"系统原生输入框"降级方案 |
| R5 | egui 默认字体无中文 | 中文显示为方块 | 内嵌 Noto Sans SC 子集（3-5 MB），启动时注册到 `FontDefinitions` |
| R6 | 混合 DPI 多屏坐标错乱 | 截图区域偏移/模糊 | 引入 VSS 物理像素统一坐标系；遮罩按显示器分窗口；Windows 强制 PerMonitorV2 |
| R7 | 显存/纹理泄漏 | 多次截图后显存不释放 | 纹理与 Surface 的生命周期严格绑定窗口；窗口销毁后立即 drop；加一个"已截图 N 次 / 当前显存占用"的调试面板用于验证 |
| R8 | 全屏独占游戏/DX 应用截不到 | 黑屏或旧内容 | 用 WGC 而非 BitBlt；HDR 场景需要额外做色调映射（非 MVP 范围，先记录） |
| R9 | 导出结果与预览不一致 | 用户投诉"所见非所得" | 导出走同一条 GPU 渲染管线 + readback，不复用任何独立的光栅化代码 |
| R10 | egui 每帧重建 mesh 的 CPU 开销 | 大量画笔点时掉帧 | 描边点做 Douglas-Peucker 抽稀；静态时不做重绘（只在有输入时 `request_redraw`） |
| R11 | macOS 应用公证（Notarization） | 用户下载后无法直接打开 | 需要 Apple Developer 账号、codesign + notarytool；产出 Universal Binary（x86_64 + aarch64） |
| R12 | 开机自启在某些环境被安全软件拦截 | 自启失效 | 提供自启开关的状态自检；Windows 用 HKCU Run（用户级，无需管理员）；macOS 用 LaunchAgent，并提供卸载脚本 |

---

## 8. 性能与内存预算

### 8.1 内存预算表

| 场景 | 目标 RSS | 构成 |
|---|---|---|
| 常驻托盘（Idle） | ≤ 25 MB | winit 事件循环 + egui Context + 托盘 + 热键 + 预热捕获会话 |
| 遮罩态（双 4K） | ≤ 180 MB | 两屏 BGRA 位图 2×33 MB = 66 MB（CPU 侧） + GPU 纹理 66 MB（独立显存，不占 RSS） + 框架开销 |
| 遮罩态（单 1080p） | ≤ 60 MB | 8 MB 位图 + 框架开销 |
| 5 个贴图窗口（各 1080p） | ≤ 60 MB | 每窗 GPU 纹理 8 MB |

**内存优化清单**

1. **保留 CPU 侧原图 vs 上传后释放**：上传 GPU 纹理后立刻 `drop` CPU 缓冲，可省 66 MB，但导出需要 GPU→CPU readback（增加 ~30 ms 导出延迟）。**建议做法：保留 CPU 原图（Arc 共享，不额外拷贝），因为 66 MB 的峰值是可接受的，而 readback 会增加导出延迟和复杂度。** 若实测内存吃紧，再切换到 readback 方案。
2. **缓冲区池**：捕获线程维护一个 `Vec<FrameBuffer>` 池，截图结束不释放而是归还（避免每次截图都做大块内存分配，减少 malloc 抖动和内存碎片）。
3. **不存位图快照**：撤销栈只存矢量操作（见 4.4）。
4. **可选 mimalloc**：`mimalloc` 作为全局分配器，能明显降低多线程场景下的碎片与峰值。
5. **字体只加载一份**：利用 `include_bytes!` 静态嵌入，egui 内部只栅格化实际用到的字形，不做全字库预渲染。

### 8.2 编译优化配置

```toml
[profile.release]
opt-level      = 3
lto            = "fat"
codegen-units  = 1
strip          = "symbols"
panic          = "unwind"     # 保留 unwind：单窗口 panic 不应导致整进程退出
```

- `panic = "abort"` 可显著减小体积并去掉 unwind 表，但会牺牲 `catch_unwind`。对于 GUI 应用建议**保留 unwind**（一个面板 panic 不应该整进程崩溃），因此改为 `panic = "unwind"` + `strip = true`。
- Windows 上可额外用 `/OPT:REF /OPT:ICF` 链接器优化。

### 8.3 冷启动优化

- 延迟初始化：托盘图标、设置读取、字体注册并行进行
- 捕获会话预热放在**后台线程**，不阻塞托盘图标出现
- 目标：双击图标 → 托盘可见 < 300 ms

---

## 9. 目录结构

```
snipaste-rs/
├── Cargo.toml                  # workspace 定义 + release profile
├── rust-toolchain.toml         # 固定工具链版本
├── crates/
│   ├── app/
│   │   ├── src/
│   │   │   ├── main.rs             # 入口：单实例检查、EventLoop、run_app
│   │   │   ├── app.rs              # ApplicationHandler 实现
│   │   │   ├── state.rs            # Mode 状态机
│   │   │   ├── tray.rs             # 托盘与菜单
│   │   │   ├── overlay.rs          # 遮罩窗口管理（建/毁/绘制）
│   │   │   ├── pin.rs              # 贴图窗口管理
│   │   │   ├── settings_ui.rs      # 设置窗口
│   │   │   └── config.rs           # 配置加载/保存（TOML）
│   │   └── build.rs                # Windows manifest / macOS Info.plist 注入
│   ├── core/
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── geometry.rs         # Pos/Rect/变换矩阵
│   │   │   ├── document.rs         # Document / Selection
│   │   │   ├── annotation.rs       # Annotation 枚举与命中测试
│   │   │   ├── history.rs          # Edit / History
│   │   │   ├── style.rs            # Style / 色板 / 预设
│   │   │   └── simplify.rs         # Douglas-Peucker / Catmull-Rom
│   │   └── tests/                  # 纯逻辑单元测试
│   ├── render/
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── theme.rs            # egui 主题（浅色/深色）
│   │   │   ├── toolbar.rs          # 工具栏控件
│   │   │   ├── tessellate.rs       # Annotation → egui Mesh
│   │   │   ├── mosaic.rs           # 马赛克 shader + pipeline
│   │   │   └── fonts.rs            # CJK 字体注册
│   │   └── shaders/mosaic.wgsl
│   ├── capture/
│   │   ├── src/
│   │   │   ├── lib.rs              # CaptureBackend trait
│   │   │   ├── session.rs          # 会话池 + 预热 + 帧回收
│   │   │   └── convert.rs          # 各种像素格式 → BGRA8
│   │   └── ...
│   └── platform/
│       ├── src/
│       │   ├── lib.rs              # 统一 trait：Hotkey/Clipboard/Autostart/WindowExt
│       │   ├── windows/            # WGC、RegisterHotKey、Run 注册表、DPI
│       │   ├── macos/              # SCK、Carbon HotKey、LaunchAgent、TCC、NSWindow
│       │   └── common/             # 无平台差异的实现
│       └── ...
├── assets/
│   ├── fonts/NotoSansSC-Subset.ttf
│   └── icons/                      # 应用图标、托盘图标（Win: .ico, mac: .icns）
├── resources/
│   ├── windows/app.manifest        # PerMonitorV2 DPI + asInvoker
│   └── macos/Info.plist            # LSUIElement、版本号、URL Scheme
├── docs/
│   ├── architecture.md             # 本文档
│   ├── api-design.md               # （后续）对外接口与扩展点
│   └── build.md                    # （后续）构建、签名、公证流程
└── README.md
```

---

## 10. 里程碑与排期建议

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **M0 骨架** | Rust 工具链就位；workspace 建好；winit + egui-wgpu 出一个空白窗口；托盘图标 + 菜单；全局热键注册 F1；开机自启开关 | 打包成 exe/app 能自启，托盘可见，按 F1 能收到事件日志 |
| **M1 截屏闭环** | 捕获后端（Win 用 WGC，mac 用 SCK）；遮罩窗口（多屏）；框选；尺寸提示；`Esc` 取消；复制到剪贴板；另存为 | 双屏 4K 下 `F1` → 遮罩 ≤ 150 ms；框选后 `Ctrl+C` 能粘到微信/PS |
| **M2 标注核心** | 矩形、椭圆、箭头、画笔、记号笔、文字、橡皮擦；撤销/重做；工具栏；颜色/粗细选择；**中文 IME 实测通过** | 13 项功能中 1/2/3/4/6/7/8/9/10/12 完成；中文输入无方块、无崩溃 |
| **M3 高级功能** | 马赛克（GPU）；钉在桌面（多窗口、拖动、缩放、透明度、吸附）；二维码识别；导出与预览一致性校准 | 13 项功能全部完成；导出与预览逐像素一致 |
| **M4 打磨与发布** | 内存/性能达标；设置窗口；热键自定义；崩溃日志；Windows 打包（NSIS/MSI）+ 签名；macOS 打包（Universal .app）+ 公证 | 空载内存 ≤ 25 MB；两平台安装包可直接分发给用户 |

**建议节奏**：M0 → M1 是风险最高的一段（平台 API 和权限），务必先打通再往上堆功能。

---

## 11. 打包与发布

> **⚠️ 现状（M0 结束时）**：本节内容**全部尚未实施**。
> 目前没有任何打包脚本 / 安装器 / 图标 / 代码签名 / 自启注册，
> 产物就是 `target/.../release/snipaste-rs.exe` 这个**裸 exe**。
> 下面的清单是 M4 阶段的工作项。

### Windows

**实测基线（2026-10-11）**

| 项 | 值 |
|---|---|
| 裸 exe | **11.72 MiB** |
| zip deflate | 5.36 MiB |
| xz preset=9 | 4.06 MiB |

- **不需要 `+crt-static`**。实测该 exe 只依赖 Windows 系统 DLL
  （`objdump -p` 确认无 `libgcc_s_seh-1.dll` / `libwinpthread-1.dll`），
  **已经是可直接分发的单文件**。初稿"需静态链接 CRT"的说法对本工具链不成立。
- 工具：`cargo-wix`（MSI）或 NSIS 脚本
- 代码签名：EV 证书或普通 OV 证书（无签名会被 SmartScreen 拦截）
- Manifest 必填：`dpiAwareness = PerMonitorV2`、`requestedExecutionLevel = asInvoker`、`longPathAware`

### macOS

> **本机无法产出 macOS 产物**。已装 `aarch64-apple-darwin` 与 `x86_64-apple-darwin`
> target，`cargo check --target aarch64-apple-darwin` **全绿**（跨端设计成立），
> 但链接需要 Apple SDK 与链接器（`-framework` / `-arch` 是 clang 专有参数），
> 在 Windows 上必然失败。**必须用 Mac 或 GitHub Actions 的 `macos-latest` runner。**
>
> 参照 Windows 的 11.72 MiB，单架构（arm64）二进制预计在 **10-13 MiB** 量级，
> Universal Binary 约翻倍。**这是估算，不是实测**，需要出包后回填真实数字。

- 工具：`cargo-bundle` 或手写 `.app` 目录结构
- **Universal Binary**：`cargo build --target x86_64-apple-darwin` + `--target aarch64-apple-darwin` 然后 `lipo -create`
- `Info.plist` 关键项：
  - `LSUIElement = true`（隐藏 Dock 图标，纯托盘应用）
  - `LSMinimumSystemVersion = 12.3`
  - `NSHighResolutionCapable = true`
  - `CFBundleIdentifier`、`CFBundleShortVersionString`
- 签名与公证：`codesign --deep --options runtime` → `xcrun notarytool submit` → `xcrun stapler staple`

---

## 12. 后续可扩展方向（不在本期范围）

- 滚动截图（长截图）：连续捕获 + 图像拼接
- 录屏 / GIF 录制（`xcap` 已具备雏形）
- OCR 文字识别（`tesseract` / `paddleocr` 绑定）
- 条码识别（换用 `rxing`）
- 贴图历史库（本地图库 + 搜索）
- 取色器 / 屏幕标尺（Snipaste 的配套小工具）
- 标注样式模板与预设（保存常用色板）
- 多语言界面（i18n）

---

## 附录 A：MVP 最小可运行切片（建议的第一个 PR）

如果要把风险最低地启动项目，第一个 PR 只做这一件事：

```
winit 窗口（全屏、无边框、置顶）
  + egui-wgpu 渲染
  + 显示一张硬编码的纯色背景 + 一个跟随鼠标的矩形
  + 按 Esc 退出
```

它能一次性验证：winit 0.30 的 `ApplicationHandler` 写法、egui-wgpu 的集成方式、CJK 字体注册、release 构建的内存与体积基线。这四件事是后面所有工作的地基。

## 附录 B：需要尽早动手验证的 5 个实验

| 实验 | 目的 | 若失败的后果 |
|---|---|---|
| E1：WGC 在双 4K 下取帧耗时 | 验证 150 ms 目标可行性 | 需降级 GDI + 提前捕获策略 |
| E2：macOS SCK 首次授权流程 + 重启生效性 | 验证权限引导体验 | 需要设计更长的引导流程 |
| E3：egui TextEdit 在 Win/mac 中文 IME 预编辑 | 验证 4.6 方案 | 需要启用手写 IME 或兜底输入框 |
| E4：多屏混合 DPI 下遮罩窗口的坐标精度 | 验证 VSS 设计 | 需要改回跨屏单窗口方案 |
| E5：Noto Sans SC 子集后的二进制体积与显示效果 | 验证体积目标 | 需要改用系统字体（Win 雅黑 / mac 苹方）+ 运行时加载 |

---

## 附录 C：M0 骨架实测记录（2026-10-11）

本节记录 M0「最小可运行切片」的**真实**落地结果。初稿的 API 认知基于文档与经验，
实际编码时在 egui 0.36 / wgpu 30 上遇到大量**破坏性 API 变更**，这里逐条列出，
后人在写捕获层与标注层时可直接对照，不必再翻遍源码。

### C.1 交付物清单

| 项 | 状态 | 说明 |
|---|---|---|
| Rust 工具链 | ✅ | 1.99.0 / `stable-x86_64-pc-windows-gnu` |
| cargo workspace 骨架 | ✅ | 5 个 crate，见 §3.3 |
| winit 全屏无边框置顶窗口 | ✅ | `ApplicationHandler` 模式 |
| egui-wgpu 渲染管线 | ✅ | 自建 `Surface` + `RenderPass` |
| CJK 字体注册 | ✅ | 运行时探测系统字体，已加载「等线 Deng」 |
| 系统托盘 + 菜单 | ✅ | 截图/贴到桌面/开机自启/退出 |
| 全局热键 | ✅ | F1（本机被占用，自动降级） |
| 单元测试 | ✅ | **103 个全通过**（core 80 / app 15 / render 8） |
| 端到端运行 | ✅ | 60 fps 稳定，无 panic，无显存泄漏 |

### C.2 egui 0.36 的破坏性变更（初稿未覆盖）

| 变更 | 初稿写法 | 实际写法 |
|---|---|---|
| `Context::style()` / `set_style()` 移除 | `ctx.set_style(style)` | `ctx.all_styles_mut(\|s\| { ... })` |
| 根 `Ui` 取代 `CentralPanel` | `CentralPanel::default().show(ctx, ...)` | `ctx.run_ui(...)` 提供根 `Ui` |
| `Button` 用 `AtomLayout` 重写 | 依赖 `override_text_color` 回退 | **必须**显式 `.color(fg)`，否则按钮只有框没有字 |
| `Align2::anchor_size` 返回类型变了 | 当作 `Pos2` 用 | 返回 `Rect`，用 `.min` 取位置 |

> **踩坑记录**：工具条按钮初版只有外框、没有文字，排查半天才发现是 `Button`
> 重写后不再回退到 `override_text_color`。这个坑很隐蔽，务必在自定义主题时先验证。

### C.3 wgpu 30 的破坏性变更（初稿未覆盖）

| 变更 | 初稿写法 | 实际写法 |
|---|---|---|
| `InstanceDescriptor` 不再 `Default` | `Default::default()` | `InstanceDescriptor::new_without_display_handle()` |
| 能力查询改名 | `surface.get_surface_capabilities(&a)` | `surface.get_capabilities(&a)` |
| 取帧返回枚举而非 `Result` | `let tex = surface.get_current_texture()?` | match `Success / Suboptimal / Timeout / Occluded` |
| `present` 移到 `Queue` | `tex.present()` | `queue.present(tex)` |
| `RenderPassDescriptor` 新字段 | 无 | 必须补 `multiview_mask: None` |
| `textures_delta.set` 变为双层 | `for (id, delta) in set` | `for (id, deltas) in set { for d in deltas }` |

> **踩坑记录**：egui 的 `TexturesDelta` 有 **drop 断言** —— 若未把其中的
> 纹理增量全部 apply 并 `clear()` 就 drop，会直接 panic：
> `Dropped TexturesDelta with N unapplied deltas`。
> 正确写法是 `std::mem::take` 出来，在函数出口统一 `free_texture` + `clear()`。

### C.4 其他实际结论

| 结论 | 说明 |
|---|---|
| sRGB 格式会告警 | 从 `caps.formats` 里优先挑 `Bgra8Unorm` / `Rgba8Unorm`（**非** sRGB），否则 egui 报线性 framebuffer 警告 |
| `Color32` 是**预乘**存储 | 取回原色要用 `to_srgba_unmultiplied()`；`r()/g()/b()` 给的是预乘分量。且 8 位量化有 ±1 误差，测试要用容差 |
| `Viewport` 抽象是必要的 | 构造函数用 `Viewport::new(origin, scale)` 统一 VSS 物理像素 ↔ egui 逻辑点，非法 scale 回退 1.0 |
| 对象级橡皮擦需要索引映射 | 擦除拖动期间 `items` 在变，必须用 `alive` 表把索引映射回**涂抹开始时**的索引，松手时合并成**一条** `Edit::RemoveMany` |
| 本机必须关闭增量编译 | Windows 文件锁异常 → `.lock` 删不掉报 os error 5；**并发** cargo 命令还会写坏缓存导致 rustc ICE。**必须写在 `Cargo.toml` 的 `[profile.dev] incremental = false`**；`.cargo/config.toml` 的 `[env] CARGO_INCREMENTAL` **无效**（cargo 启动后 `[env]` 才生效，rustc 仍收到 `-C incremental`），详见 `docs/build.md` §5.4 |

> **真实缺陷（已修）**：橡皮擦初版只处理 `[last, p]` 线段，松手时**最后一段没擦到**；
> 且每次 `extend` 都登记一条历史，一次涂抹产生多条撤销记录。
> 修法：引入 `erased` / `alive` 字段 + `erase_segment()` / `flush_erase()`。
> 这个 bug 是单元测试抓出来的，**验证了"先写测试"的价值**。

### C.5 全局热键的实际约束（重要）

`RegisterHotKey` 是**进程独占**的。本机因装有正版 Snipaste，**F1 已被占用**，
注册直接返回 `HotKey already registered: F1`。这不是 bug，而是全局热键的固有特性。

因此实现上**必须**做候选降级链，而不是单点注册：

```text
F1  →  Ctrl+Shift+F1  →  Ctrl+Alt+F1  →  PrintScreen
```

本机实际生效的是 **`Ctrl+Shift+F1`**，并在状态栏向用户明示当前实际热键。

> 这条对 §7 的 R3 风险条目是**实证补充**：风险不是"将来可能发生"，而是"本机立刻发生"。
> 后续必须提供**热键自定义 UI**，并在首次启动时检测冲突、主动引导用户改键。

### C.6 性能与内存实测

| 指标 | 实测值 | 初稿预算 | 结论 |
|---|---|---|---|
| 稳定帧率 | **182~542 帧 / 3~9 秒 ≈ 60 fps** | 60 FPS | ✅ 达标 |
| release 二进制 | **11.72 MiB**（单文件自包含） | — | ✅ 体积很小 |
| debug 二进制 | 380 MiB（对照） | — | — |
| 进程内存（release） | **196.5 MiB 工作集** | 20-30 MB | ❌ **远超预算** |
| 进程内存（dev） | 约 206 MB | 20-30 MB | 同上 |

#### C.6.1 release 产物构成（Windows）

```text
snipaste-rs.exe        12,291,072 B = 11.72 MiB
  └ zip deflate         5.36 MiB（45.8%）
  └ xz preset=9         4.06 MiB（34.6%）
```

`objdump -p` 确认**只依赖 Windows 系统 DLL**（KERNEL32 / USER32 / dwmapi / dxgi /
ole32 / shcore …），**没有 `libgcc_s_seh-1.dll`、`libwinpthread-1.dll` 这类
MinGW 运行时依赖** —— 也就是说这是一个**可直接分发的单文件 exe**，
不需要附带任何 DLL。这一点是 GNU 工具链的默认行为，不必额外加 `-static`。

#### C.6.2 内存去哪了：一次模块级定位

用 `EnumProcessModulesEx` 列出进程内所有已映射模块后，结论非常明确：

| 模块 | 镜像大小 | 归属 |
|---|---|---|
| `nvgpucomp64.dll` | 94.05 MiB | NVIDIA（shader/计算 JIT 编译器） |
| `nvwgf2umx.dll` | 85.64 MiB | NVIDIA（WDDM 用户态驱动） |
| `nvrtum64.dll` | 52.39 MiB | NVIDIA（运行时 UMD） |
| `nvoglv64.dll` | 46.52 MiB | NVIDIA（OpenGL ICD） |
| `nvppex.dll` + `nvapi64_impl.dll` | 12.61 MiB | NVIDIA |
| **`snipaste-rs.exe`** | **11.75 MiB** | **本项目自身** |
| 其余（Shell32 / combase / ntdll …） | ~110 MiB | Windows 系统库 |

**结论：196 MB 里几乎没有一份是我们自己的代码**，绝大部分是 GPU 驱动的用户态
模块被映射进来（其中相当比例是与其他进程共享的页面）。但**任务管理器按进程显示**，
用户看到的会是 196 MB —— 所以「内存小」这条硬指标**在观感上确实没有达标**。

> **试过的优化（未采纳）**：把 wgpu 后端从「默认全探测」限定为 `VULKAN | METAL`，
> 工作集 196.5 → 173.1 MiB，**只省 12%**。原因是那四个 NVIDIA DLL 是 Vulkan
> 驱动自己拉起的伴生库，不受 wgpu 后端开关控制。而代价是**没有 Vulkan 的机器
> （虚拟机 / 远程桌面 / 老显卡）会直接起不来**，收益与风险不对等，**已还原**。

**后续可选方向**（需权衡，尚未决策）：

| 方向 | 预期 | 代价 |
|---|---|---|
| 改用 DX12 后端替代 Vulkan | DX12 驱动栈可能更省内存（待实测） | 需改后端选择逻辑并重测 |
| 放弃 GPU 加速（softbuffer + 自研光栅） | 内存可降至个位数 MB | 4K 双屏合成会吃掉一个核，与「性能高」冲突 |
| 只在遮罩空闲期释放 GPU 资源 | 常驻态内存降下来 | 每次 F1 要重建 device，与「≤150ms」冲突 |
| 接受现状，把指标改为「常驻托盘态 ≤ XX MB」 | 无 | 需要向用户澄清验收口径 |

> **建议**：先实测 DX12 后端的内存表现。若 DX12 能压到 100 MB 以内，
> 那就是性价比最高的方案 —— 不用牺牲性能，只换一个后端。

### C.7 常驻托盘化与选区交互（2026-10-11 第二轮）

M0 骨架跑通后暴露了三个与「常驻托盘截图工具」定位不符的问题，已修正。

#### C.7.1 启动不再自动弹遮罩

初版 `App::new` 里 `visible: true`，窗口创建即显示，还会立刻套一个 70% 内缩的
默认选区 —— 等于「打开程序就自动进入截图」。这与「常驻托盘、按 F1 才截图」的
定位直接冲突。

**修法**：窗口照常在启动时创建（GPU device、字体图集都在这一帧建好，按 F1 时
就不用再等），但**立刻 `window.set_visible(false)` 保持隐藏**。呼出入口收敛成
**唯一一个** `App::start_capture()`：

```
F1 热键 ──┐
托盘菜单 ──┼──► start_capture() ──► set_overlay_visible(true)
冒烟模式 ──┘
```

启动流程**刻意不调用它**，从而在结构上保证「打开程序不会自动截图」。
`F1` 再次按下时若遮罩已开，则收起（与 Snipaste 的 F1 行为一致）。

> 实测：无 `SNIPASTE_SMOKE_MS` 启动后 5 秒截屏，桌面干净、无遮罩无工具条；
> 日志中也没有「已进入截图」。

#### C.7.2 选区支持自由调整大小

初版 `Tool::Select` 只实现了整体平移，`paint_selection_frame` 画的 8 个角柄
**纯属装饰、点不中**。

**修法**（纯逻辑下沉到 `core`，符合「core 不含平台代码」的约束）：

| 层 | 新增 |
|---|---|
| `core/geometry.rs` | `Handle` 枚举（LT/RT/RB/LB/T/R/B/L）+ `Rect::handle_at(pos, tol)` + `Rect::resize(handle, target, min_size)` + `Rect::clamp_within(bounds)` |
| `render/canvas.rs` | 角柄改由 `Rect::handles()` / `Handle::ALL` 驱动，与命中测试**共用同一套点** |
| `app/editor.rs` | `Drag::ResizeSelection(Handle)` 与 `Drag::NewSelection`；`begin` 里按优先级分派 |

**优先级不能调换**：角柄压在选区边界上 —— 必须先判 `handle_at`，再判
`contains`（平移），最后才是「选区外 → 拖出新选区」。若先判 `contains`，
边界上的角柄就永远抓不到。

两个易错点已写成测试：

- `resize_never_flips_past_the_opposite_edge` —— 拖过头时矩形不能翻转
- `a_stray_click_outside_keeps_the_old_selection` —— 选区外只点一下没拖动，
  属误触，应**保留**原选区而不是清掉

#### C.7.3 默认选区吸附指针下的窗口

按 F1 时不再给「整屏」或「固定比例内缩」，而是**自动吸附到指针所在窗口的边框**
（Snipaste 的实际行为）。

**实现**：`crates/platform/src/window.rs` 新增 `window_under_point`。

```text
WindowFromPoint(指针)
  └─ GetAncestor(.., GA_ROOT)          提升到顶层窗口（指针常命中的是子控件）
  └─ 过滤：IsWindowVisible / DwmGetWindowAttribute(DWMWA_CLOAKED)
            / 进程 PID == 自己（不能吸附到自己的遮罩）
            / 窗口类名落在「桌面本身」黑名单（Progman / WorkerW / Shell_TrayWnd…）
  └─ DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)   取**肉眼可见**的边框
       └─ 失败则退回 GetWindowRect
  └─ 过滤：最小化时 Win32 会把窗口丢到 (-32000, -32000)，这种坐标必须丢弃
```

用 `DWMWA_EXTENDED_FRAME_BOUNDS` 而不是 `GetWindowRect` 的原因：Win10+ 的
不可见窗口阴影/拉伸边框会比肉眼看到的窗口大一圈，直接截图会带进一圈背景色。

**探针与去抖**：吸附发生在 UI 层的探针里（需要 egui 提供的指针位置）。
`arm_snap()` 打开后，指针移动超过阈值才重新查询窗口 —— 避免高频重查。

> 实测：按 F1 后选区标签显示 **`1200 × 800`**，正是指针下那个窗口的真实尺寸，
> 说明吸附生效（整屏或固定比例都不会出现这个数字）。

#### C.7.4 本轮验收

| 项目 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 无差异 |
| `cargo clippy --workspace --all-targets` | **0 告警** |
| `cargo test --workspace` | **121 通过 / 0 失败**（M0 的 103 + 新增 18） |
| 冒烟 | **481 帧 / 8 秒 ≈ 60 fps**，正常退出 |
| 启动态截屏 | 桌面干净，无遮罩（常驻托盘成立） |
| 呼出态截屏 | 压暗遮罩 + 工具条正常，选区吸附到 `1200 × 800` 窗口 |
| 热键 | 本轮成功注册为 **F1**（占用 F1 的那个程序当时未运行） |

### C.8 「选区调不了大小」的根因排查（第二轮修复）

C.7 交付后用户实测反馈：**「现在不能自由调整截图的选择」**。C.7.2 写的角柄
缩放代码和 18 个单元测试都通过了，但用户**就是拖不动**。

排查结论：真正的原因有 4 个，全部位于「单元测试覆盖不到」的地方。
其中 **第 1 条是主因**，后 3 条是同一批被顺带挖出来的真实缺陷。

#### C.8.1 【主因】默认工具是「矩形」而不是「指针」

`Document::new()` 里写的是 `tool: Tool::Rect`。

于是用户按下 F1、看到选区、想拖角柄调整大小时，**按下鼠标变成了画一个红色矩形**，
选区纹丝不动 —— 复现的正是用户的反馈。

为什么单元测试没抓到：`editor.rs` 的测试辅助函数里写着

```rust
fn selection_editor() -> Editor {
    let mut ed = Editor::new();
    ed.set_tool(Tool::Select);   // ← 这一行把真实默认值盖掉了
    ...
}
```

所有角柄测试都在一个**用户根本遇不到的状态**（工具=指针）下跑，自然全绿。

**修法**：`Document::new()` 改成 `Tool::Select`，并把测试辅助函数里的
`set_tool` 删掉、改成 `assert_eq!(ed.doc.tool, Tool::Select)` —— 让测试反映
「程序启动时的真实默认」，而不是测试作者以为的默认。

> **这条的教训比 bug 本身重要**：测试里**不要手动摆好被测状态**，
> 尤其是"用户走一遍就会到达的状态"。测试应该从真实初值出发。

#### C.8.2 角柄命中用了「当前位置」而不是「按下点」

egui 判定「这是拖拽而不是点击」要等指针离开按下点超过
`InputOptions::max_click_dist`（0.36 里是 **6.0 点**）。所以
`Response::drag_started()` 触发时，`interact_pointer_pos()` **已经离开按下点
至少 6px**（斜向 ≈8.5px）。

原来 `begin(view.to_vss(resp.interact_pointer_pos()))` 拿这个「已在移动途中」
的位置去判角柄，命中点整体偏出 6~10px，正好把角柄的命中半径挤到边缘或外面：

- 偏离后仍落在选区内 → 退化成「整体平移」（看起来"选区在动但改不了大小"）
- 偏离后落到选区外 → 退化成「新建选区」，**用户调好的选区直接被清掉**

**修法**：改用 `ui.input(|i| i.pointer.press_origin())`，即真正的按下点。

#### C.8.3 指针一按下就该退出吸附，不能等 `drag_started()`

吸附探针的去抖步长 `SNAP_PROBE_STEP` 恰好也写在 **6.0**，与 egui 的
`max_click_dist` 撞在一起。于是在「已按下、但还没被判定成拖拽」的那几帧里：
指针已移动 ≥6px → 探针触发 → **把用户正要拖的选区整个换成指针下窗口的矩形**。

表现就是「一拖，选区就跳回去」。

**修法**：
1. 指针一按下（`pointer.any_pressed()`）就 `disarm_snap()`；
2. 吸附块的条件从 `!resp.dragged()` 改成 `!pointer.any_down()` —— 后者才是
   「指针根本没按下」的准确表达。

#### C.8.4 `drag_started()` 那一帧的位移被整段吞掉

`drag_started()` 真正触发时，指针已经离开按下点 ≥6px。原代码在那一帧只调
`begin`，位移要等下一帧的 `dragged()` 才通过 `extend` 生效 —— 这一帧的运动
全丢了。拖得慢看不出来，**快速甩动（一帧跨几十像素）就会觉得"拖了没反应"**。

**修法**：`begin` 之后立刻用当前位置补一次 `extend`。

**同类缺陷**：`Editor::finish()` 里 `Drag::MoveSelection` 是**空分支**
（`Drag::Idle | Drag::MoveSelection => {}`），而 `Shape` / `Freehand` /
`Erase` / `ResizeSelection` 都会在 `finish` 里落位。于是「按下—移动—松手」
发生在同一帧时平移整段丢失。已补上。

#### C.8.5 角柄的「画」和「点」终于共用一个常量

- 渲染侧 `render/canvas.rs`：`const HANDLE_SIZE: f32 = 8.0;`
- 命中侧 `app/editor.rs`：`hit_tolerance() + HANDLE_HIT_SLOP(6.0)` = 12.0

两套数字各写一份，改动时必然漂移。

**修法**：提升到 `core/geometry.rs`，由渲染与命中共同引用：

```rust
pub const HANDLE_SIZE: f32 = 10.0;      // 角柄边长（物理像素）
pub const HANDLE_HIT_SLOP: f32 = 5.0;   // 本体之外的外扩容差
pub const HANDLE_HIT_RADIUS: f32 = HANDLE_SIZE * 0.5 + HANDLE_HIT_SLOP;
```

角柄因此从 8 点放大到 10 点、可抓范围 20px 见方，4K 屏上明显更好瞄。

#### C.8.6 整屏选区不参与平移

修好默认工具后出现的新问题：呼出遮罩时选区默认是**整屏**（还没选），
此时在它内部按下会走 `Drag::MoveSelection` —— 平移一个铺满屏幕的矩形，
屏幕上什么都不会变，看起来就是「点了没反应」。

**修法**：`selection_is_full_screen()` 为真时跳过平移分支，直接进入「重新框选」。
语义上很自然：**整屏选区不是真选区，只是「还没选」的占位**。

#### C.8.7 补上的测试（这类缺陷必须用无头 egui 才测得到）

新增 `crates/app/src/ui.rs` 的测试模块：直接用 `Context::run_ui` 把**合成的
指针事件**喂进去，覆盖完整的「按下 → 移动 → 松手」时序。

```rust
let mut out = ctx.run_ui(input, |ui| { draw(ui, ed, view, None, "F1") });
out.textures_delta.clear();   // ⚠️ 不 clear 会触发 drop 断言 panic
```

| 测试 | 抓的缺陷 |
|---|---|
| `corner_handle_is_grabbed_from_the_press_point_not_the_latest_pointer` | C.8.2 |
| `pointer_down_disarms_window_snapping_immediately` | C.8.3 |
| `pressing_well_inside_the_selection_moves_it` | C.8.4 |
| `dragging_inside_a_full_screen_selection_starts_a_new_one` | C.8.6 |
| `a_window_selection_is_still_moved_not_rebuilt` | C.8.6 的反向边界 |

**已验证这 5 个测试在旧代码上会失败**（把三处修复临时改回去跑了一遍，全红），
所以它们是真回归测试，不是事后凑的。

#### C.8.8 端到端验证（真实鼠标事件）

单元测试只覆盖到 `ui.rs`，再往下（winit 事件 → egui 输入 → 真实渲染）就断了。
为此补了两个脚本，用 Win32 合成鼠标事件（`SetCursorPos` + `mouse_event`）
真实地拖一次角柄：

| 脚本 | 做法 | 结果 |
|---|---|---|
| `scripts/e2e-drag.py` | 关吸附；用「压暗遮罩亮度≈原亮 53%」这一特性，**在截图上量出选区边界** | 左边界 60→804、上边界 8→608（期望 800/600）✅ |
| `scripts/e2e-snap.py` | 开吸附（用户的真实流程）；**同时**读程序日志的选区矩形 + 从截图做差量出遮罩认定的选区，两路交叉验证 | 吸附到真实窗口 `1200×800`，拖右下角柄后 `→1457×982`：左/上边界纹丝不动，右边界 +257、下边界 +182 ✅ |

为支持脚本驱动，新增环境变量：

- `SNIPASTE_NO_SNAP=1` —— 关闭窗口吸附。除了测试，它也是**给用户的逃生开关**：
  吸附本质是启发式，遇到无边框全屏窗口 / 游戏会给出意外的选区。
- `Editor` 在选区变化与拖拽结束时打 `log::debug!`，供脚本读取（也方便用户报障）。

#### C.8.9 本轮验收

| 项目 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 无差异 |
| `cargo clippy --workspace --all-targets` | **0 告警** |
| `cargo test --workspace` | **126 通过 / 0 失败**（C.7 的 121 + 新增 5） |
| `scripts/e2e-drag.py` | 拖拽角柄生效 ✅ |
| `scripts/e2e-snap.py` | 吸附态下拖拽角柄生效，**逻辑层与渲染层偏差仅 3px** ✅ |

#### C.8.10 把「截图证据自相矛盾」查到底

C.8.8 第一版验证留了个尾巴：日志明确说拖拽生效，但拿截图去核对亮度时，
得到的却是**整屏都被压暗**（选区内亮度 135 / 选区外 132），与日志直接冲突。

没有把它当"渲染 bug"草草定性，而是先怀疑度量方法本身，逐个排除：

1. **截图时机** —— 松手后 1.2s / 3.2s 各拍一张，两张逐像素完全一致
   （被压暗像素都是 2,562,237，占比 51.7%）。**排除瞬时帧**。
2. **基准图污染** —— clean 基准图改到**启动程序之前**拍。冒烟模式会自己进截图态，
   拍晚了基准图里就带着遮罩，做差得到一屏黑。
3. **真正的原因：判据错了**

   原来用「绝对灰度差 > 18 ⇒ 被压暗」。问题是桌面壁纸、终端窗口、任务栏这些
   **本来就暗**的像素，压暗 47% 后差值也到不了 18：

   | 原始亮度 | 压暗后 | 差值 | 是否计入 |
   |---|---|---|---|
   | 250 | 132 | 118 | ✅ |
   | 27 | 14 | 13 | ❌ **漏判** |

   这些暗像素被错归为"未压暗"，于是"未压暗区包围盒"扩散到整屏
   `(0,23)-(3440,1435)`，**看上去就像整屏被压暗**。
   判据的缺陷伪装成了渲染的缺陷。

改成**相对判据** `shot <= clean * 0.80`（且 `clean >= 70` 以排除噪声），
再逐行/逐列找最长连续段取中位数（比包围盒鲁棒——个别噪点拉不歪矩形）：

| | 日志（逻辑层） | 像素（渲染层） | 偏差 |
|---|---|---|---|
| 吸附后 | `(1168,162)-(2368,962)` = 1200×800 | `(1169,161)-(2365,961)` = 1196×800 | 3px |
| 拖拽后 | `(1168,162)-(2625,1144)` = 1457×982 | `(1169,161)-(2622,1141)` = 1453×980 | 3px |

3px 正是遮罩描边 + 抗锯齿的固有宽度。**两路独立证据吻合，矛盾消除。**

沉淀成一条给后续所有自动化测试的规矩：
**验证渲染结果永远用相对判据，不要用绝对阈值**——屏幕内容亮度方差极大，
绝对阈值必然在暗色区域失效，而且失效方式是"看起来像渲染错了"。
