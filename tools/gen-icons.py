#!/usr/bin/env python3
"""生成 Veil 的两套 Windows 图标。

唯一真源
--------
  ``icons/Markdown.png``  文档图标设计稿（256×256）
  本文件的几何参数        应用图标「白页 + 面纱垂坠 V」

产物（直接入库，构建期由 ``crates/veil/build.rs`` 以 ``include_bytes!`` 嵌入）
-------------------------------------------------------------------------------
  ``icons/veil.ico``       exe 资源 ID 1：应用图标（标题栏 / 任务栏 / 资源管理器）
  ``icons/veil-doc.ico``   exe 资源 ID 2：文档图标（``.md`` 文件关联）
  ``icons/veil-32.rgba``   窗口图标：32×32 原始 RGBA，供 winit ``with_window_icon``

用法
----
    python tools/gen-icons.py

依赖 Pillow + numpy。改动设计参数后重跑本脚本即可，无需改 Rust 代码
（``build.rs`` 已登记 ``rerun-if-changed``）。
"""

from __future__ import annotations

import io
import struct
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "icons"
DOC_SOURCE = ICONS / "Markdown.png"

# —— 配色（描边与墨色取样自文档图标，保证两套图标同族）——
INK = (17, 21, 26, 255)          # #11151A
BORDER = (181, 188, 199, 255)    # #B5BCC7
BORDER_SOFT = (221, 226, 233, 255)
FILL = (255, 255, 255, 255)

# —— 应用图标几何（1024 基准，按目标尺寸等比缩放）——
TILE_INSET = 0.055     # 圆角方块内缩比例
TILE_RADIUS = 0.128    # 圆角半径占画布比例
BORDER_W = 0.0042      # 描边宽占画布比例
V_TOP, V_BOTTOM = 263.0, 733.0   # V 的起笔与汇合纵坐标（墨迹包围盒垂直居中于画布）
V_DX = 200.0           # 单臂水平偏移
V_STROKE = 80.0        # 笔画宽度（≈21px@256，与设计稿一致）
SS = 16                # 超采样倍数

SIZES = [16, 20, 24, 32, 48, 64, 256]
SMALL_DOC_MAX = 24     # ≤24px 改用简化徽标，避免 M↓ 笔画粘连


def resize_premultiplied(image: Image.Image, size: int) -> Image.Image:
    """预乘 alpha 缩放：避免透明区（RGB=黑色）在缩放时产生深色毛边。"""
    src = np.array(image, dtype=np.float32)
    alpha = src[..., 3:4] / 255.0
    premultiplied = src.copy()
    premultiplied[..., :3] = src[..., :3] * alpha

    resized = Image.fromarray(
        np.clip(premultiplied, 0, 255).astype(np.uint8), "RGBA"
    ).resize((size, size), Image.LANCZOS)

    out = np.array(resized, dtype=np.float32)
    a = out[..., 3:4] / 255.0
    rgb = np.where(a > 1e-4, out[..., :3] / np.maximum(a, 1e-4), 255.0)
    return Image.fromarray(
        np.concatenate([np.clip(rgb, 0, 255), out[..., 3:4]], axis=2).astype(np.uint8),
        "RGBA",
    )


def draw_tile(draw: ImageDraw.ImageDraw, canvas: int, inset: float,
              radius: float, border_w: float, border_color) -> None:
    """白色圆角方块 + 描边（用两层圆角矩形画环）。"""
    pad = inset * canvas
    rad = radius * canvas
    draw.rounded_rectangle([pad, pad, canvas - 1 - pad, canvas - 1 - pad],
                           radius=rad, fill=border_color)
    draw.rounded_rectangle([pad + border_w, pad + border_w,
                            canvas - 1 - pad - border_w, canvas - 1 - pad - border_w],
                           radius=max(1.0, rad - border_w), fill=FILL)


def draw_app_icon(size: int) -> Image.Image:
    """应用图标：白页 + 面纱垂坠 V。"""
    canvas = size * SS
    scale = canvas / 1024.0
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    border_color = BORDER if size >= 32 else BORDER_SOFT
    draw_tile(draw, canvas, TILE_INSET, TILE_RADIUS,
              max(BORDER_W * canvas, 1.0 * SS), border_color)

    center = canvas / 2.0
    stroke = V_STROKE * scale
    if size < 32:
        stroke = max(stroke, 1.35 * SS)  # 小尺寸下限，防止 V 过细
    draw.line(
        [(center - V_DX * scale, V_TOP * scale),
         (center, V_BOTTOM * scale),
         (center + V_DX * scale, V_TOP * scale)],
        fill=INK, width=int(round(stroke)), joint="curve",
    )
    return resize_premultiplied(image, size)


def draw_doc_icon_small(size: int) -> Image.Image:
    """文档图标小尺寸：白底徽标 + 加粗 M（去掉箭头，避免笔画粘连）。"""
    canvas = size * 32
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    draw_tile(draw, canvas, 0.045, 0.20,
              max(0.0045 * canvas, 1.0 * 32), BORDER_SOFT)

    x0, x1 = 0.30 * canvas, 0.70 * canvas
    top, bottom = 0.30 * canvas, 0.70 * canvas
    middle = (x0 + x1) / 2
    stroke = max(0.105 * canvas, 1.25 * 32)
    draw.line([(x0, bottom), (x0, top), (middle, bottom), (x1, top), (x1, bottom)],
              fill=INK, width=int(round(stroke)), joint="curve")
    return resize_premultiplied(image, size)


def encode_ico(frames: list[tuple[int, Image.Image]]) -> bytes:
    """写 ICO：≥256 用 PNG 帧，其余用 32bpp DIB + AND 掩码。"""
    payloads: list[bytes] = []
    for size, image in frames:
        if size >= 256:
            buf = io.BytesIO()
            image.save(buf, format="PNG", optimize=True)
            payloads.append(buf.getvalue())
            continue

        header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0,
                             size * size * 4, 0, 0, 0, 0)
        pixels = image.load()
        body = bytearray()
        mask = bytearray()
        mask_row_bytes = ((size + 31) // 32) * 4
        for y in range(size - 1, -1, -1):
            row = bytearray(mask_row_bytes)
            for x in range(size):
                r, g, b, a = pixels[x, y]
                body += bytes((b, g, r, a))
                if a < 128:
                    row[x // 8] |= 0x80 >> (x % 8)
            mask += row
        payloads.append(header + bytes(body) + bytes(mask))

    out = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
    offset = 6 + 16 * len(frames)
    for (size, _), payload in zip(frames, payloads):
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(payload), offset)
        offset += len(payload)
    for payload in payloads:
        out += payload
    return bytes(out)


def main() -> None:
    if not DOC_SOURCE.is_file():
        raise SystemExit(f"缺少文档图标设计稿：{DOC_SOURCE}")

    doc = Image.open(DOC_SOURCE).convert("RGBA")
    if doc.size != (256, 256):
        raise SystemExit(f"文档图标设计稿应为 256×256，实际 {doc.size}")

    app_frames = [(size, draw_app_icon(size)) for size in SIZES]
    doc_frames = []
    for size in SIZES:
        if size >= 32:
            doc_frames.append((size, doc.copy() if size == 256
                               else resize_premultiplied(doc, size)))
        else:
            doc_frames.append((size, draw_doc_icon_small(size)))

    (ICONS / "veil.ico").write_bytes(encode_ico(app_frames))
    (ICONS / "veil-doc.ico").write_bytes(encode_ico(doc_frames))
    (ICONS / "veil-32.rgba").write_bytes(draw_app_icon(32).tobytes())

    for name in ("veil.ico", "veil-doc.ico", "veil-32.rgba"):
        print(f"{name:14s} {(ICONS / name).stat().st_size:>7d} B")


if __name__ == "__main__":
    main()
