"""端到端验证：吸附态下拖角柄缩放选区（也就是用户的真实操作路径）。

    F1 呼出 → 指针移到某个窗口上（选区自动吸附）→ 拖右下角柄放大选区

判定用两条互相独立的证据链交叉验证：

  A. **逻辑层**：程序自己打印的选区矩形（`snipaste_rs::editor` 的 debug 日志）。
  B. **渲染层**：把"遮罩弹出前"的 clean 图和"遮罩弹出后"的实拍图做差，
     量出遮罩实际压暗了哪块地方，反推它认定的选区。

只信 A 等于"程序自己说自己对了"；只信 B 分不清是渲染错还是截图方式错。
两者吻合，结论才成立。

⚠️ 量选区**不能**用固定灰度差阈值。桌面壁纸、终端窗口、任务栏这些本来就暗的
像素，压暗 47% 后差值也到不了阈值，会被误判成"未压暗"，于是"未压暗区包围盒"
扩散到整个屏幕，得出"整屏被压暗"的假结论（这个坑真踩过）。
必须用**相对判据** `shot <= clean * RATIO`，再逐行/逐列找最长连续段取中位数。

用法：
    <venv-python> scripts/e2e-snap.py <exe 路径>
"""

import ctypes
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image, ImageGrab

EXE = sys.argv[1]
OUT = Path("target")
OUT.mkdir(exist_ok=True)

u32 = ctypes.windll.user32
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004

HOVER = (1700, 700)  # 指针停这里 → 吸附到该处的窗口
GROW = (260, 180)  # 从右下角往外拖这么多
W, H = 3440, 1440  # 选区会被钳制在这个范围内
RATIO = 0.80  # shot/clean 低于此值 ⇒ 判定为被压暗
MIN_LUMA = 70  # clean 亮度低于此值不参与判定（噪声太大）

RECT_RE = re.compile(
    r"min: Pos \{ x: ([\d.-]+), y: ([\d.-]+) \}, max: Pos \{ x: ([\d.-]+), y: ([\d.-]+) \}"
)


def move(x, y, settle=0.03):
    u32.SetCursorPos(int(x), int(y))
    time.sleep(settle)


def read_rects(runs):
    out = []
    for r in runs:
        out += [tuple(float(v) for v in m) for m in RECT_RE.findall(r)]
    return out


def grab():
    return ImageGrab.grab(all_screens=False, include_layered_windows=True).convert("RGB")


def snapped_shot():
    """加一小段稳定等待，避开合成过程中的瞬时帧。"""
    time.sleep(0.35)
    return grab()


def _longest_run(seq, gap=6):
    """最长连续 1 段，允许中间有小空洞（选区内的文字、控件会造成空洞）。"""
    best = cur = 0
    best_end = cur_end = 0
    skip = 0
    for i, v in enumerate(seq):
        if v:
            cur += 1 + skip
            skip = 0
            cur_end = i
        elif skip < gap:
            skip += 1
        else:
            if cur > best:
                best, best_end = cur, cur_end
            cur = cur_end = skip = 0
    if cur > best:
        best, best_end = cur, cur_end
    return best, best_end


def measure(clean, shot, tag, scale=0.32):
    """量出遮罩认定的选区，返回 (x0, y0, x1, y1)。同时落盘可视化掩码。"""
    a, b = clean.convert("L"), shot.convert("L")
    assert a.size == b.size, f"尺寸变了：{a.size} vs {b.size}"
    w, h = a.size
    cp = list(a.getdata())
    sp = list(b.getdata())
    lit = [
        255 if (c >= MIN_LUMA and s > c * RATIO) else 0
        for c, s in zip(cp, sp, strict=True)
    ]
    Image.frombytes("L", (w, h), bytes(lit)).resize(
        (int(w * scale), int(h * scale))
    ).save(OUT / f"mask-{tag}.png")

    xs = []
    for y in range(h):
        n, end = _longest_run(lit[y * w : (y + 1) * w])
        if n > w * 0.3:
            xs.append((end - n + 1, end))
    ys = []
    for x in range(w):
        n, end = _longest_run(lit[x::w])
        if n > h * 0.3:
            ys.append((end - n + 1, end))

    if not xs or not ys:
        return None
    xs.sort()
    ys.sort()
    x0, x1 = xs[len(xs) // 2]
    y0, y1 = ys[len(ys) // 2]
    return (x0, y0, x1, y1)


def main():
    subprocess.run(
        ["taskkill", "/F", "/IM", "snipaste-rs.exe"], capture_output=True, check=False
    )
    time.sleep(1.0)

    # ① 基准图必须在启动程序**之前**拍。冒烟模式会自己进截图态，
    #    拍晚了基准图里就带上遮罩了，做差会得到一屏黑。
    clean = grab()
    clean.save(OUT / "snap-clean.png")
    print(f"[0] 基准图 clean 已拍 {clean.size}")

    env = dict(os.environ, SNIPASTE_SMOKE_MS="60000")
    env.pop("SNIPASTE_NO_SNAP", None)  # 确保吸附开着
    env["RUST_LOG"] = "info,snipaste_rs::editor=debug"

    log_path = OUT / "snap.log"
    log = open(log_path, "w")
    proc = subprocess.Popen([EXE], env=env, stdout=log, stderr=subprocess.STDOUT)

    try:
        time.sleep(6)
        move(*HOVER)  # 指针停到目标窗口上 → 吸附探针改写选区
        time.sleep(2.0)
        log.flush()

        text = log_path.read_text(encoding="utf-8", errors="replace")
        picks = [
            ln for ln in text.splitlines() if "选区 = " in ln and "拖拽结束" not in ln
        ]
        if not picks:
            print("!! 日志里没有选区记录，无法判读。日志尾部：")
            print("\n".join(text.splitlines()[-25:]))
            return
        before = read_rects([picks[-1]])[0]
        print(f"[1] 吸附后的选区（日志  ）= {before}")
        print(f"    {picks[-1].split('] ')[-1]}")

        shot_before = grab()
        shot_before.save(OUT / "snap-before.png")

        # ② 拖右下角柄。分 40 步走，模拟真实拖拽产生的连续 motion 事件；
        #    一步到位等于"瞬移"，测不出拖拽逻辑。
        rx, ry = before[2], before[3]
        move(rx - 3, ry - 3, settle=0.6)
        u32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
        time.sleep(0.2)
        for i in range(1, 41):
            move(rx - 3 + GROW[0] * i / 40, ry - 3 + GROW[1] * i / 40)
        time.sleep(0.4)
        u32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
        time.sleep(2.5)
        log.flush()

        shot_after = snapped_shot()
        shot_after.save(OUT / "snap-after.png")
        time.sleep(2.0)
        shot_after2 = snapped_shot()
        shot_after2.save(OUT / "snap-after2.png")

        ends = [
            ln
            for ln in log_path.read_text(encoding="utf-8", errors="replace").splitlines()
            if "拖拽结束" in ln
        ]
        if not ends:
            print("!! 日志里没有拖拽结束记录")
            return
        after = read_rects([ends[-1]])[0]
        print(f"[2] 拖动后的选区（日志  ）= {after}")
        print(f"    {ends[-1].split('] ')[-1]}")

        # ③ 渲染层证据
        print()
        print("---- 渲染层证据（clean 做差量出遮罩认定的选区；掩码图白=选区）----")
        for tag, shot, ref in (
            ("before", shot_before, before),
            ("after", shot_after, after),
            ("after2", shot_after2, after),
        ):
            got = measure(clean, shot, tag)
            if got is None:
                print(f"[{tag:6}] 量不出矩形")
                continue
            d = max(abs(got[i] - ref[i]) for i in range(4))
            print(
                f"[{tag:6}] 像素量出 = ({got[0]}, {got[1]}) - ({got[2]}, {got[3]})"
                f"   尺寸 {got[2] - got[0]} x {got[3] - got[1]}"
                f"   ｛与日志最大偏差 {d:.0f}px｝{'OK' if d <= 12 else 'FAIL'}"
            )

    finally:
        proc.kill()
        subprocess.run(
            ["taskkill", "/F", "/IM", "snipaste-rs.exe"], capture_output=True, check=False
        )
        log.close()

    print()
    # 选区会被钳制在屏幕内，所以"外扩"的期望值是 min(目标, 屏幕边缘)
    exp_r = min(before[2] + GROW[0], W)
    exp_b = min(before[3] + GROW[1], H)
    ok_l = abs(after[0] - before[0]) <= 1.0
    ok_t = abs(after[1] - before[1]) <= 1.0
    ok_r = abs(after[2] - exp_r) <= 12
    ok_b = abs(after[3] - exp_b) <= 12
    print(f"左边界不动  {before[0]:.0f} → {after[0]:.0f}  {'OK' if ok_l else 'FAIL'}")
    print(f"上边界不动  {before[1]:.0f} → {after[1]:.0f}  {'OK' if ok_t else 'FAIL'}")
    print(
        f"右边界外扩  {before[2]:.0f} → {after[2]:.0f}  (期望 {exp_r:.0f}"
        f"{'，被屏幕钳制' if exp_r < before[2] + GROW[0] else ''})  {'OK' if ok_r else 'FAIL'}"
    )
    print(
        f"下边界外扩  {before[3]:.0f} → {after[3]:.0f}  (期望 {exp_b:.0f}"
        f"{'，被屏幕钳制' if exp_b < before[3] + GROW[1] else ''})  {'OK' if ok_b else 'FAIL'}"
    )
    print(
        "=> 吸附态下拖角柄生效"
        if all([ok_l, ok_t, ok_r, ok_b])
        else "=> 未生效"
    )


main()
