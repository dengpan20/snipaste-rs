# macOS release 内存优化记录

日期：2026-10-12。环境：Apple M3 MacBook Air（24 GB）、macOS 15.5、Rust 1.99.0、macOS 15.4 SDK。前后均为 0.2.4 release 构建，截图和测试贴图尺寸为 **5120 × 2880**。

两次测量均由 LaunchServices 独立启动签名 `.app`，使用正式应用的 bundle identifier 和本机证书，沿用既有授权，未从终端继承录屏权限。测量执行三轮截图 / 四种强度马赛克 / 全屏标注导出，然后创建一张、三张全尺寸纯色测试贴图并关闭，等候空闲资源释放，未写入剪贴板。

## 实测结果

单位为 **MiB（1,048,576 字节）**。Mac 的 CSV `private_bytes` 记录 physical footprint；Windows 上该字段是另一种指标，不能直接比较。resident size 包含当前驻留页面，与 footprint 含义不同。

| 指标 | 修改前 | 修改后 |
|---|---:|---:|
| 启动待机 footprint | 10.5 | 10.9 |
| 第一轮截图完成首帧后的 footprint | 511.2 | 388.1 |
| 第三轮截图结束、释放资源后 footprint | 18.0 | 14.0 |
| 三张贴图阶段采样 footprint | 597.9 | 545.8 |
| 贴图全部关闭、释放资源后 footprint | 21.5 | 14.4 |
| 全流程累计 peak footprint | 1150.8 | 810.7 |
| 全流程累计 peak resident size | 761.7 | 472.0 |

本轮 peak footprint 降低约 **30%**，peak resident size 降低约 **38%**；贴图关闭后的 footprint 降低约 **33%**。启动待机基本不变，主要收益是活动会话和结束后的占用。

原始数据：[修改前 CSV](macos-memory-before.csv)、[修改后 CSV](macos-memory-after.csv)。每份有 17 条采样，包含屏幕尺寸、贴图数量和 GPU / 截图窗口状态。结束后无 GPU、截图窗口或贴图。原始目录为 `dist/macos-memory/before/` 与 `dist/macos-memory/after/`。

这是一次完整配对测量。不同桌面内容、图像透明度、窗口尺寸、内存压力和驱动缓存会影响结果，尤其是即时采样；累计峰值包括阶段之间异步绘制的分配。跨屏尝试保留于 `after-mixed-display/`，未纳入对比。修改后指定 5120 × 2880 屏幕，修改前每个活动阶段也都是该尺寸。

三轮首帧准备耗时为修改前 **665 / 289 / 330 ms**、修改后 **315 / 234 / 238 ms**。系统和着色器缓存状态存在差异，不能据此承诺固定加速比例。

## 实现变化

- **只读取所用字体。** 本机苹方 TTC 含 24 个字面、78,201,856 字节；egui 字体后端还会复制字体数据。现在按所用字面的表目录直接读取约 13 MiB，保留完整字形、可变字体和排版表，不限制可输入字符。重建偏移与校验和，去掉失效的集合签名；无法提取时回退到原始读取。[集合格式依据](https://learn.microsoft.com/en-us/typography/opentype/spec/otff)
- **贴图菜单按需加载字体。** 仅显示图片时不加载中文字体；首次右键按下时，在菜单首帧之前加载。
- **减少整屏临时副本。** 上传纹理后即释放 CPU 更新数据；首帧像素的所有权转交 AppKit 的图像提供者，省去 `CFDataCreate` 的整屏复制，图像释放时通过 Rust 回调归还内存。[图像提供者 API](https://developer.apple.com/documentation/coregraphics/cgdataprovider/init%28datainfo%3Adata%3Asize%3Areleasedata%3A%29?language=objc)
- **归还空闲堆缓存。** 字体变小后，分配器保留的空闲块曾使待机 footprint 增加约 12 MiB。现在仅在无截图、无贴图、GPU 已释放时调用 `malloc_zone_pressure_relief`，归还已经释放的缓存；不将仍使用的截图或字体换出内存。[Apple 接口说明](https://github.com/apple-oss-distributions/libmalloc/blob/main/include/malloc/malloc.h)

5K 冻结底图、贴图原始像素和 Metal 窗口缓冲仍需内存。本次保留原始分辨率、中文输入和导出质量。

## 验证与复测

`bash scripts/build-macos.sh --verify` 通过 fmt / clippy、**150 项测试**、原生桥编译、release 构建与签名。新增测试覆盖字面选择、绝对表偏移、表数据和校验和、越界 / 截断 / 重复表 / 异常分配大小，以及首帧创建失败时只释放所有权一次。

`dist/macos-memory/native-gui/result.txt` 为 `PASS`：覆盖实际捕获、连续截图首帧、取色、标注导出、二维码、剪贴板、贴图和资源生命周期；额外验证首次右键前不加载中文字体、右键后菜单中文字形齐全。已查看界面截图，中文标注和工具栏文字正常。功能验收使用 debug 构建，内存对比使用上述 release 构建。

复测（先退出映贴，验收应用需有可用录屏权限）：

```bash
SNIPASTE_MEMORY_PROFILE_SCREEN=5120x2880 bash scripts/verify-macos.sh --memory-pins --release
```

没有 5K 屏时替换为实际物理尺寸，或省略该变量以固定首次截图的屏幕。脚本从终端启动，权限可能归属于宿主，不能据此判断正式应用的独立授权。详见 [Mac 指南](macos.md)。
