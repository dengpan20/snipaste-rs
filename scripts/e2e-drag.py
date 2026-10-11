"""端到端验证：用合成的真实鼠标事件拖动选区左上角柄，程序化量出选区变化。

不是项目代码，只是验证脚本（target/ 已被 git 忽略）。

原理：选区之外的压暗遮罩是 47% 黑，所以「未压暗区域」的亮度约为原亮的 53%。
先截一张**真正没有遮罩**的干净桌面作基准（必须在启动程序之前截！），
再截遮罩态，逐块比较亮度比值，就能精确算出选区边界 —— 不用靠肉眼看图。

注意：必须用 `SNIPASTE_NO_SNAP=1` 运行被测程序。否则窗口吸附探针会在指针
移向角柄的过程中不断改写选区，脚本根本没法把指针停在角柄上。

用法：
    python e2e_drag.py <exe 路径>
"""

import ctypes
import os
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image, ImageGrab

EXE = sys.argv[1]
OUT = Path("target")

u32 = ctypes.windll.user32

MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004

W, H = 3440, 1440
TARGET_X, TARGET_Y = 800, 600


def move(x, y, settle=0.03):
    u32.SetCursorPos(int(x), int(y))
    time.sleep(settle)


def mouse_down():
    u32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)


def mouse_up():
    u32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)


def grab():
    return ImageGrab.grab(all_screens=False, include_layered_windows=True).convert("RGB")


def block_mean(img, x, y, r=6):
    box = (max(0, x - r), max(0, y - r), x + r, y + r)
    px = list(img.crop(box).convert("L").getdata())
    return sum(px) / max(1, len(px))


def dim_ratio(clean, shot, x, y, r=6):
    """(x, y) 附近一块的平均亮度比 shot/clean。1.0 = 没被压暗。"""
    ca = block_mean(clean, x, y, r)
    if ca < 22:  # 基准太暗，比值没意义
        return None
    return block_mean(shot, x, y, r) / ca


def first_not_dimmed(clean, shot, *, fixed, axis, rng, thresh=0.85):
    """沿一条扫描线找第一处"没被压暗"的位置。"""
    for v in rng:
        x, y = (v, fixed) if axis == "x" else (fixed, v)
        r = dim_ratio(clean, shot, x, y)
        if r is not None and r > thresh:
            return v
    return None


def main():
    # 先清掉任何残留实例：两个全屏置顶遮罩会互相遮挡，让结果无法解释
    subprocess.run(
        ["taskkill", "/F", "/IM", "snipaste-rs.exe"], capture_output=True, check=False
    )
    time.sleep(1.0)

    # ① 基准必须在启动程序之前截：冒烟模式会在 ~1.5s 后自动进入截图态
    clean = grab()
    clean.save(OUT / "e2e-clean.png")
    print("[0] 干净桌面基准已截（程序尚未启动）")

    env = dict(os.environ, RUST_LOG="info", SNIPASTE_SMOKE_MS="60000", SNIPASTE_NO_SNAP="1")
    log = open(OUT / "e2e.log", "w")
    proc = subprocess.Popen([EXE], env=env, stdout=log, stderr=subprocess.STDOUT)

    try:
        time.sleep(6)
        before = grab()
        before.save(OUT / "e2e-before.png")

        left_b = first_not_dimmed(clean, before, fixed=1000, axis="x", rng=range(60, W, 8))
        top_b = first_not_dimmed(clean, before, fixed=3200, axis="y", rng=range(8, H, 8))
        print(f"[1] 呼出后选区左上角 ≈ ({left_b}, {top_b})   预期 (0, 0)")

        # ② 拖左上角柄。分 40 步移动，模拟真实拖拽（不是一步跳过去）。
        move(3, 3, settle=0.6)
        mouse_down()
        time.sleep(0.2)
        for i in range(1, 41):
            move(3 + (TARGET_X - 3) * i / 40, 3 + (TARGET_Y - 3) * i / 40)
        time.sleep(0.4)
        mouse_up()
        time.sleep(1.2)

        after = grab()
        after.save(OUT / "e2e-after.png")

        left_a = first_not_dimmed(clean, after, fixed=1000, axis="x", rng=range(60, W, 8))
        top_a = first_not_dimmed(clean, after, fixed=3200, axis="y", rng=range(8, H, 8))
        print(f"[2] 拖动后选区左上角 ≈ ({left_a}, {top_a})   预期 ({TARGET_X}, {TARGET_Y})")
    finally:
        proc.kill()
        subprocess.run(
            ["taskkill", "/F", "/IM", "snipaste-rs.exe"],
            capture_output=True,
            check=False,
        )
        log.close()

    # 全图均值：只要选区变了，被压暗的面积就会变，均值必然跟着动
    print()
    for name in ("e2e-clean", "e2e-before", "e2e-after"):
        g = Image.open(OUT / f"{name}.png").convert("L")
        px = list(g.getdata())
        print(f"  {name:14} 全图亮度均值 {sum(px) / len(px):7.2f}")

    print()
    ok_x = left_a is not None and abs(left_a - TARGET_X) <= 32
    ok_y = top_a is not None and abs(top_a - TARGET_Y) <= 32
    print(f"左边界  {left_b} → {left_a}   (期望 → {TARGET_X})  {'OK' if ok_x else 'FAIL'}")
    print(f"上边界  {top_b} → {top_a}   (期望 → {TARGET_Y})  {'OK' if ok_y else 'FAIL'}")
    print("=> 拖拽角柄生效" if (ok_x and ok_y) else "=> 拖拽角柄未生效")


main()
