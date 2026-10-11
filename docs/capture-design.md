# capture 层详细设计

> 对应 `crates/capture`
> 上游文档：`docs/architecture.md`
> 版本：v1.0 · 2026-10-11

---

## 1. 职责边界

**负责**

- 把"屏幕上的像素"变成内存里的 `FrameBuffer`
- 管理跨显示器的并行取值
- 管理捕获会话的预热与释放
- 把各平台的原始像素格式**归一化**成统一格式
- 读取并上报显示器拓扑（分辨率、位置、缩放、主屏）

**明确不负责**

| 不负责 | 归属 |
|---|---|
| 窗口创建、事件循环、遮罩 | `app` |
| 帧上传 GPU、纹理管理 | `render` / `app` |
| 标注、撤销、导出合成 | `core` / `app` |
| 权限弹窗 UI | `app`（`capture` 只**检测并上报**权限状态） |
| 全局热键、自启、剪贴板 | `platform` |

这条边界很重要：`capture` 不依赖 `winit`、不依赖 `egui`、不依赖 `wgpu`。它是一个纯粹的
"像素来源"，可以被单元测试、可以被离线基准测试、甚至可以被编成一个命令行小工具来验证。

---

## 2. 核心抽象

### 2.1 后端 trait

```rust
/// 每个平台提供一个实现。由专门的捕获线程独占持有，因此无需内部加锁。
pub trait CaptureBackend: Send {
    /// 后端名。日志、设置页、诊断面板都会展示它，便于定位"用户到底走的哪条路"。
    fn name(&self) -> &'static str;

    /// 预热：把固定开销前置到进程启动阶段。
    ///
    /// 这是达成「F1 按下 → 遮罩出现 ≤ 150 ms」的关键。首次建立捕获会话
    /// 通常有 50~200 ms 的一次性成本，如果放在 F1 触发时才做，预算直接爆掉。
    ///
    /// 实现应当**幂等**：重复调用不重复建立会话。
    fn warm_up(&mut self, monitors: &[MonitorInfo]) -> Result<(), CaptureError>;

    /// 显示器拓扑发生变化（插拔屏幕、改分辨率、改缩放）时调用。
    ///
    /// Windows 上需要重建 WGC 会话；macOS 上需要让缓存的 `SCShareableContent` 失效。
    fn on_display_change(&mut self, monitors: &[MonitorInfo]) -> Result<(), CaptureError>;

    /// 捕获全部指定显示器。实现内部负责并行。
    fn capture_all(&mut self, monitors: &[MonitorInfo]) -> Result<Vec<MonitorShot>, CaptureError>;

    /// 释放会话与缓存。
    ///
    /// 进入长时间空闲时调用，目的是把常驻内存降回 ~25 MB 目标。
    /// 默认实现什么都不做，适合无会话状态的后端（如 CGDisplayCreateImage）。
    fn release(&mut self) {}
}
```

**为什么是 `&mut self` 而不是 `&self`**：Tauri 系的一些捕获库用 `&self` + 内部 `Mutex`，
结果是每次取帧都要过一把锁，且锁的粒度很难判断。本方案把这个后端**整体移动到一个
专门的捕获线程**里，通过 channel 通信，因此可以用 `&mut self`，零锁、零原子操作。

### 2.2 帧缓冲

```rust
/// 一帧像素。用 Arc 传递，避免在跨线程、跨模块传递时复制像素。
#[derive(Debug, Clone)]
pub struct FrameBuffer {
    pub width: u32,
    pub height: u32,
    /// 行跨度（字节）。**可能大于 `width * bytes_per_pixel`**，见下文说明。
    pub stride: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

/// 全流程统一到 BGRA8 —— 理由见 §4。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 每像素 4 字节，通道顺序 B, G, R, A。Windows 侧的原生格式。
    Bgra8,
    /// 每像素 4 字节，通道顺序 R, G, B, A。
    Rgba8,
}

pub struct MonitorShot {
    /// 该帧属于哪块屏（含 VSS 位置与缩放倍率）。
    pub info: MonitorInfo,
    /// 像素数据。`Arc` 共享，向上层传递只做原子计数 +1。
    pub frame: Arc<FrameBuffer>,
}
```

### 2.3 ⚠️ 关于 `stride`：这是最容易踩的坑

GPU 纹理的行是**按 4 字节或 256 字节对齐**的。一块 1920 宽的 BGRA 图，每行
1920 × 4 = 7680 字节，看起来是 4 的倍数、没问题；但换成 1366 宽就是 5464 字节，
仍可能因为对齐要求被填充到 5504。

Windows Graphics Capture 给出的 `ID3D11Texture2D` 映射后，
`D3D11_MAPPED_SUBRESOURCE::RowPitch` **经常大于** `width * 4`。

如果直接 `Vec::from_raw_parts(row_pitch * height)` 然后按 `width * 4` 去读，
画面会出现**逐行斜切的扭曲**（斜条纹）——这是这类工具最经典的 bug 之一。

**对策**：所有后端在产出 `FrameBuffer` 时**必须**显式设置 `stride`，
所有消费方（上传 GPU、裁剪、导出）**必须**按 `stride` 逐行取，不允许假设
`stride == width * 4`。转换函数集中在 `convert.rs` 一处，并配单元测试覆盖
非对齐宽度（1366、1918、111 这类"坏数字"）。

---

## 3. 会话预热机制

### 3.1 为什么必须预热

`F1` → 遮罩出现 的 150 ms 预算拆解（双 4K）：

| 阶段 | 冷启动耗时 | 预热后 |
|---|---|---|
| 建立捕获会话（设备、会话对象、首帧） | 50 ~ 200 ms | 0（已就绪） |
| 取一帧 | 10 ~ 25 ms / 屏 | 10 ~ 25 ms / 屏 |
| 上传 GPU 纹理 | 10 ~ 25 ms | 10 ~ 25 ms |
| 创建遮罩窗口 + 首帧 egui 布局 | 30 ~ 60 ms | 30 ~ 60 ms |
| **合计** | **100 ~ 310 ms** ❌ | **50 ~ 110 ms** ✅ |

结论：**不预热就一定无法稳定达标**。

### 3.2 预热时机

```
进程启动
  ├─ 托盘图标出现（用户可见，< 300 ms）        ← 不能被捕获初始化阻塞
  └─ 后台线程：
       ├─ 枚举显示器
       ├─ 选择后端（探测 WGC 是否可用）
       ├─ warm_up()
       └─ 上报状态（就绪 / 失败 / 需要权限）
```

关键点：**预热是后台线程的事，不能拖慢托盘出现**。用户看到图标就有了"程序已经起来"
的反馈，此时捕获还在后台准备。

### 3.3 预热失败的处理

`warm_up` 失败不应该是致命的。可能原因与对策：

| 原因 | 对策 |
|---|---|
| macOS 未授权屏幕录制 | 记录状态，托盘菜单显示引导项；授权后重建 |
| Windows < 1903（无 WGC） | 自动降级到 DXGI Desktop Duplication |
| 虚拟机 / 远程桌面（无 D3D 设备） | 降级到 GDI `BitBlt` |
| 显示器拓扑读取失败 | 延迟到首次 `capture_all` 时重试 |

因此 `AppEvent` 中需要有一个 `CaptureStatus` 事件，让 UI 能如实反映"当前能不能截图"。

---

## 4. 像素格式：全流程统一 BGRA8

**决策：所有后端产出的帧，一律归一化成 `PixelFormat::Bgra8`。**

理由是两个平台的原生格式恰好都是 BGRA，归一化到 BGRA8 意味着**两端都是零转换**：

| 平台 | 原生格式 | 到 BGRA8 的成本 |
|---|---|---|
| Windows WGC | `DXGI_FORMAT_B8G8R8A8_UNORM` | **零拷贝** —— 直接用 |
| Windows GDI | `BI_RGB` DIB（实际是 BGRX） | 零拷贝（把 X 当 A=255） |
| macOS SCScreenshotManager | `CGImage` + `kCGBitmapByteOrder32Little` + `kCGImageAlphaPremultipliedFirst` | **零转换** —— CGBitmapContext 声明这个组合，输出就是 BGRA |
| macOS CGDisplayCreateImage | 同上 | 同上 |

反过来，如果选 RGBA8，两端都要做一次全图通道交换 —— 双 4K 就是 6600 万次字节交换，
白白多花 10~30 ms，还多一次全图内存写。

### 4.1 需要显式处理的例外

| 情况 | 处理 |
|---|---|
| HDR 显示器（WGC 返回 `R16G16B16A16_FLOAT`） | 检测到就降级：让 WGC 输出 SDR，或做一次简化的 Reinhard 色调映射。**MVP 阶段先记为已知限制**，检测到即在日志告警 |
| macOS `SCStream` 的 `420v`（双平面视频范围） | 本方案不用 SCStream，规避该格式 |
| 行跨度填充 | 见 §2.3，逐行拷贝剥离填充 |

### 4.2 `convert.rs` 的职责

```rust
/// 把带行跨度的原始缓冲压成紧凑排列（去掉行尾填充）。
pub fn compact_strided(src: &[u8], width: u32, height: u32, stride: u32, bpp: usize) -> Option<Vec<u8>>;

/// BGRA ↔ RGBA 就地交换（仅在没有零转换路径的平台上用到）。
pub fn swap_rb_in_place(buf: &mut [u8]);

/// 把一行 BGRX（GDI 的 32 位 DIB）的 X 通道补成 255。
pub fn fill_alpha_in_place(buf: &mut [u8]);
```

这三个函数是纯字节操作，**必须 100% 单测覆盖**，包括奇数宽度、行跨度为 0、
缓冲长度不足等边界。它们出错的症状（斜条纹、颜色错位、透明变黑）在 GUI 上
肉眼可见但极难定位到根因。

---

## 5. Windows 实现

### 5.1 后端选型链

```
WindowsGraphicsCapture (WGC)        首选，Win10 1903+
    ↓ 不可用
DXGI Desktop Duplication            备选，Win8+
    ↓ 不可用
GDI BitBlt                          兜底，任何版本，但慢且拿不到独占全屏
```

探测顺序在 `warm_up` 里完成一次，结果缓存，后续不再重复探测。

### 5.2 WGC 的两个必须处理的问题

**问题一：黄色捕获边框**

WGC 默认会在被捕获对象周围画一圈黄色边框。对"截个图就消失"的工具来说，
这个边框会一闪而过，观感很差。

- Windows 11：通过 `IGraphicsCaptureSession3::SetIsBorderRequired(false)` 关闭
- Windows 10：**无法关闭**。对策是在屏幕上不可见时取值 —— 但用户按 F1 时窗口已可见，
  所以 Win10 上必然会有边框。

**这直接决定了预热策略的一个细节**：Win10 上不能"预热后直接取缓存帧"，因为缓存帧
是带边框的旧画面。正确做法是预热只建会话、不缓存像素，F1 时现取一帧。
边框只出现在捕获的瞬间，配合"遮罩立即覆盖全屏"，用户几乎察觉不到。

> 这条需要实测确认。若 Win10 上边框仍然明显，退路是改用 DXGI Desktop Duplication
> 作为 Win10 的默认后端（DD 没有边框问题），把 WGC 限定在 Win11+。

**问题二：显示器拓扑变化**

改分辨率、插拔屏幕、切换刷新率都会让 `Direct3D11CaptureFramePool` 尺寸失配。
必须监听 `WM_DISPLAYCHANGE` 并调用 `on_display_change` 重建会话，否则会出现
画面被拉伸或只显示左上角一块。

### 5.3 多屏并行

每块显示器一个独立的 `GraphicsCaptureItem` + `IDirect3DDevice`。

**为什么每屏一个 D3D 设备，而不是共用一个**：D3D11 的 immediate context
**不是线程安全的**。共用设备就必须在取帧时加锁串行化，那就不叫并行了。
每屏一个设备，各线程完全独立，代价是每设备约 1~3 MB（≤4 块屏完全可接受）。

替代方案（后续可选优化）：共用设备 + 每线程一个 deferred context。
收益不大、复杂度明显上升，暂不采用。

### 5.4 取帧的同步语义

WGC 是**推模式**：帧通过 `FrameArrived` 事件异步送达。

对截图工具而言需要的是"现在这一帧"，因此有两种实现：

| 方案 | 做法 | 评价 |
|---|---|---|
| **主动轮询式** | `TryGetNextFrame()` 循环直到拿到一帧新帧 | ✅ 简单、可控、延迟稳定 |
| 事件回调式 | 注册 `FrameArrived`，回调里存帧 | ❌ 需要额外的同步原语，且回调时机不可控 |

选**主动轮询式**。配合一个硬超时（如 300 ms），超时即以 `CaptureError::Timeout` 上报，
绝不无限等待 —— 一个卡死的截图工具会让用户以为电脑死机了。

---

## 6. macOS 实现

### 6.1 后端选型链

```
SCScreenshotManager.captureImage   首选，macOS 14+
    ↓ 不可用
CGDisplayCreateImage               fallback，macOS 10.15+（已废弃但可用）
```

### 6.2 关键决策：**不使用 `SCStream`**

`SCStream` 是持续的帧推流，需要维护会话、需要预热、需要处理背压。
但截图工具**只要一帧**。macOS 14 引入的 `SCScreenshotManager.captureImage` 正是
为此设计的：一次调用、一张 `CGImage`、典型 30 ~ 80 ms。

选它的收益：

- 无需维护长时间运行的会话 → 空载内存更低
- 无需处理背压与丢帧 → 代码显著更简单
- 无需关心 `SCStreamConfiguration` 的一堆参数

代价：macOS 12.3 ~ 13 上不可用。这些版本回退到 `CGDisplayCreateImage`
（已废弃，但 API 稳定、无需权限变更，实测 20 ~ 50 ms，性能反而更好）。

**结论**：macOS 上的"预热"策略与 Windows 不同 —— 主要预热的是
`SCShareableContent` 这层元数据缓存，而不是像素会话。

### 6.3 `SCShareableContent` 缓存

`SCShareableContent.getShareableContent()` 首次调用较慢（枚举所有窗口与显示器），
但它的结果在显示器拓扑不变时可以复用。

策略：
- 首次调用后缓存，并记录显示器拓扑指纹（各屏的 id + 尺寸 + 位置）
- 每次捕获前比对指纹，不一致则重新获取
- 监听 `NSApplicationDidChangeScreenParametersNotification` 主动失效缓存

### 6.4 权限（TCC）—— 首次体验的关键节点

```rust
/// 只查询，不弹窗。用于启动时的静默预检。
extern "C" { fn CGPreflightScreenCaptureAccess() -> bool; }

/// 触发系统授权弹窗，返回是否已授权。
extern "C" { fn CGRequestScreenCaptureAccess() -> bool; }
```

**必须写进产品逻辑的三条**

1. **启动时静默预检**，未授权不要立刻弹窗打扰用户，只在托盘菜单上标记状态。
2. 用户**第一次按 F1** 时才走授权引导 —— 此时意图明确，弹窗不唐突。
3. **授权后通常必须重启进程才生效**。这是 macOS 的已知行为（TCC 在进程启动时
   注入授权状态）。因此引导流程的最后一步必须是"重启应用"按钮，
   而不是"已授权，请再试一次"——后者会让用户反复按 F1 却始终黑屏。

> macOS 15+ 的部分场景下授权可以即时生效，但**不能依赖**。统一按"重启才生效"
> 设计流程，最坏情况只是让用户多点一次重启按钮。

### 6.5 坐标系

`CGDisplayCreateImage(displayID)` 返回的图是**该显示器自己的局部坐标系**，
原点在左上角。

因此：`MonitorInfo.vss_rect.min` = 该屏在 VSS 中的偏移，
`MonitorInfo.scale_factor` = `CGDisplayPixelsWide() / NSScreen.frame.width`。

注意 macOS 的坐标原点在左下角（Cocoa 体系），而 VSS 约定原点在左上角，
转换时需要翻转 y。**这个翻转只在读取显示器拓扑时做一次**，之后全流程都是左上原点，
不要把翻转逻辑散布到各处。

---

## 7. 帧缓冲池

### 7.1 为什么需要池

双 4K 一次截图要分配 2 × 3840 × 2160 × 4 ≈ 66 MB。如果每次截图都
`Vec::with_capacity` 分配再 drop，会带来两个问题：

- **分配延迟**：66 MB 的零页分配在 Windows 上约 3 ~ 10 ms，直接吃掉预算
- **内存碎片**：反复申请释放大块内存会加剧堆碎片

### 7.2 池的生命周期 —— 这条直接关系到内存指标

```rust
pub struct FramePool {
    free: Vec<FrameBuffer>,
    /// 上一次归还帧的时间戳，用于空闲回收
    last_return: Option<Instant>,
}

impl FramePool {
    /// 取出一个至少 `needed` 字节的缓冲。优先复用，不够大才重新分配。
    pub fn acquire(&mut self, width: u32, height: u32, stride: u32) -> FrameBuffer;

    /// 归还。**不清零**（清零是额外的全图写入，纯浪费）。
    pub fn release(&mut self, buf: FrameBuffer);

    /// 空闲回收：把池清空，让 RSS 降回空载目标。
    ///
    /// 必须在「截图会话结束后 N 秒无新请求」时调用，否则池会永久
    /// 驻留 66 MB —— 这会让空载内存从 25 MB 变成 90 MB，
    /// 直接违反硬性指标。
    pub fn trim(&mut self);
}
```

**回收策略**：截图会话结束（遮罩关闭、导出完成）后启动一个 60 秒的空闲计时；
期间若有新截图则重置计时；到期调用 `trim()`。60 秒的取值是"连续截图的常见间隔"
与"用户查看任务管理器的时间"之间的折中。

**池容量上限**：最多缓存 `显示器数量` 个缓冲。再多没有收益（一次截图同时最多用
N 个），只会多占内存。

### 7.3 归还不清零的理由

`Vec<u8>` 复用时保留旧内容，新帧会**整块覆盖**它（无论 `RowPitch` 是否等于
`width * 4`，都是逐行完整覆盖）。因此清零是纯粹的浪费。

唯一的例外是行尾填充区：如果 `stride > width * 4`，填充区可能是上一帧的残留。
但消费方一律按 `stride` 逐行读取且只读 `width * 4` 字节，填充区永远不参与运算，
所以仍然不需要清零。

---

## 8. 坐标换算

`capture` 的产出必须严格带 `MonitorInfo`，让上层能用 `VssToScreen` 做换算。
需要明确的是**三层坐标**：

```
VSS 物理像素          全流程的统一坐标（core 层使用）
    │
    ├─ 平台 API 坐标   Windows: 虚拟桌面物理像素（PerMonitorV2 感知下就是这个）
    │                  macOS:   CGDisplay 局部坐标（原点左上）
    │
    └─ 窗口逻辑坐标    winit / egui 使用的 DIP，= 物理像素 / scale_factor
```

`capture` 只负责把**平台 API 坐标 → VSS 物理像素**这一段做对，并且保证
`MonitorInfo.vss_rect` 与 `scale_factor` 的可信度。窗口逻辑坐标的换算属于 `render`。

**验证方法**：写一个 `--dump-monitors` 命令行开关，打印每块屏的
`id / physical_size / logical_size / scale / vss_rect`，与系统显示设置逐项比对。
混合 DPI 环境下这项验证不能跳过 —— 一个 `scale_factor` 取值错误，
最终表现是"截图区域偏移了几十像素"，而排查会花掉几个小时。

---

## 9. 错误模型

```rust
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// macOS 屏幕录制权限未授予。UI 需要走授权引导流程。
    #[error("未获得屏幕录制权限")]
    PermissionDenied,

    /// 后端在当前系统上不可用（版本过低、无 GPU、远程桌面等）。
    #[error("捕获后端不可用: {0}")]
    Unsupported(String),

    #[error("找不到显示器: {0}")]
    MonitorNotFound(u64),

    /// 取帧超时。**绝不能无限等待** —— 卡死的截图工具会让用户以为系统挂了。
    #[error("取帧超时（{0:?}）")]
    Timeout(std::time::Duration),

    #[error("后端错误: {0}")]
    Backend(String),
}
```

### 降级矩阵

| 错误 | 降级动作 |
|---|---|
| `Unsupported` | 沿选型链切下一个后端重试一次 |
| `Timeout` | 不重试，立即上报失败并让遮罩显示提示 |
| `PermissionDenied` | 触发授权引导，不降级（换后端也需要权限） |
| `MonitorNotFound` | 重新枚举拓扑后再试一次；仍失败则用主屏 |

**降级次数上限为 1**。无限降级链会把 150 ms 的预算拖到几秒，体验比直接失败更糟。

---

## 10. 测试策略

### 10.1 单元测试（无需真实屏幕）

| 目标 | 方法 |
|---|---|
| `compact_strided` | 构造带填充的合成缓冲，验证输出紧凑且像素内容不变 |
| 奇数宽度 | 1366、1918、111 —— 这些宽度最容易触发对齐假设错误 |
| `swap_rb_in_place` | 往返两次应回到原值；奇数长度缓冲不应 panic |
| `fill_alpha_in_place` | 验证只改 alpha 通道 |
| `FramePool` | acquire/release/trim 的容量与复用行为；trim 后 `free` 为空 |
| 坐标换算 | 直接复用 `core::vss` 的测试（已覆盖混合 DPI 场景） |

### 10.2 集成验证（需要真实屏幕，但可自动化）

在 `snipaste-rs` 上提供一个隐藏命令行开关：

```bash
snipaste-rs --capture-once --out ./shots       # 每块屏存一张 PNG，同时打印各阶段耗时
snipaste-rs --dump-monitors                    # 打印显示器拓扑
snipaste-rs --bench-capture --repeat 20        # 重复捕获 20 次，输出耗时分布
```

这三个开关的价值：

- 让"性能是否达标"变成可重复测量的数字，而不是靠手感
- 让 DPI / 多屏问题可以在没有 GUI 的情况下定位
- 让后续的回归有基线可比

### 10.3 必须人工确认的清单

- [ ] 混合 DPI 双屏：截图区域与实际框选区域像素级一致
- [ ] 笔记本外接屏拔插后，仍能正确截图
- [ ] Windows 10 上 WGC 的黄色边框是否可见（决定 Win10 默认后端）
- [ ] macOS 全屏应用（游戏、视频播放器）上能否正常呼出遮罩
- [ ] 远程桌面 / 虚拟机环境下的降级路径

---

## 11. 性能预算与实测要求

| 指标 | 目标 | 测量方式 |
|---|---|---|
| `warm_up` 完成时间 | 启动后 2 s 内 | 日志时间戳 |
| 单屏取帧 | ≤ 25 ms | `--bench-capture` |
| 双屏并行取帧 | ≤ 60 ms（不是 50 ms 之和） | `--bench-capture` |
| 端到端 `F1` → 遮罩可见 | ≤ 150 ms | 录屏逐帧比对（唯一可信的方法） |
| 空闲 60 s 后 RSS | ≤ 30 MB | 任务管理器 / `--dump-mem` |

**注意最后一项**：端到端延迟只能靠**录屏逐帧比对**来测，日志时间戳无法反映
窗口真正"出现在屏幕上"的时刻（中间隔着合成器与提交队列）。这是唯一能验证
"用户感知到的延迟"的方法。

---

## 12. 实现顺序建议

1. `FrameBuffer` / `PixelFormat` / `convert.rs` + 全部单元测试（纯逻辑，零风险）
2. `FramePool` + 单元测试
3. `CaptureBackend` trait + 一个"假后端"（返回合成图案），让 `app` 层可以先跑通
   完整的「F1 → 遮罩 → 框选 → 导出」链路，不被真实的平台 API 阻塞
4. Windows WGC 实现 + `--dump-monitors` 验证拓扑
5. macOS SCScreenshotManager 实现
6. 降级链与权限流程
7. 性能基准与达标调优

第 3 步里的**假后端**是整个顺序里最关键的一步：它让上层开发与平台适配完全并行，
也让端到端测试可以在 CI 上无屏幕运行。
