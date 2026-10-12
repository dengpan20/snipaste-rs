# 映贴的应用图标

应用名称：**映贴**，取“截图、贴在桌面上”之意。原项目名为 Snipaste-RS，Rust crate、可执行文件、bundle identifier 和已有自启标识继续使用原内部名称。

图标为蓝色圆角底板、青色截图框、白色图片卡片和薄荷色图钉，外部透明。原始 PNG 保存于 `packaging/macos/AppIcon.png`；`packaging/macos/AppIcon.icns` 包含 16–1024 像素的标准 macOS 图标尺寸，正式包和调试验收包都会复制它并在 `Info.plist` 中声明。菜单栏使用适配深浅色模式的单色模板图标，不显示名称文字，鼠标悬停时提示「映贴」。

使用内置 image_gen 工具生成，未使用 CLI/API fallback。最终生成提示如下：

```text
Use case: stylized-concept
Asset type: production macOS application icon, square 1024 by 1024 PNG with transparent outer background.
Primary request: design a polished original icon for 映贴, a screenshot and desktop image pinning utility. No text or letters in the icon.
Subject: a substantial rounded square deep blue tile, with a softly luminous pale cyan screenshot frame, a white floating image card with a restrained abstract teal landscape, and one small mint pushpin fastening its upper right corner. Large simple readable silhouette.
Style: refined macOS app icon with restrained three-dimensional depth, satin surfaces, crisp rounded edges, gentle highlights, subtle shadow. Front-on, centered, no perspective tilt of the overall tile. Tasteful and clear at 32 pixels.
Composition: the rounded square tile occupies about 84 percent of the canvas with a consistent transparent margin; the screenshot card is the dominant symbol; crop-frame corners surround the card with enough visual breathing space; pushpin readable without excessive detail.
Palette: deep midnight blue tile, luminous aqua crop corners, porcelain white card, mint green pin.
Constraints: genuine transparent background outside the rounded tile, preserve alpha, no mockup scene, no words, no numbers, no watermark, no Apple logo, no multiple icon variations. One completed app icon.
```

重新生成 ICNS：

```bash
bash scripts/generate-macos-icon.sh
```

该脚本仅用 macOS 的 sips 缩放已有 PNG，再通过 iconutil 打包；不需要额外图像处理依赖。
