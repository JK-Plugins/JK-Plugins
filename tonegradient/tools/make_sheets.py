"""samples の出力（PPM）をシートごとにラベル付きで並べた PNG にまとめる。

python3 tools/make_sheets.py <samples_out> <docs/samples>
"""
import sys
from collections import defaultdict
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONT_CANDIDATES = [
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc",
    "C:/Windows/Fonts/meiryo.ttc",
]


def font(size):
    for f in FONT_CANDIDATES:
        try:
            return ImageFont.truetype(f, size)
        except OSError:
            pass
    return ImageFont.load_default()


def main(src, dst):
    src, dst = Path(src), Path(dst)
    dst.mkdir(parents=True, exist_ok=True)
    sheets = defaultdict(list)
    for p in sorted(src.glob("*.ppm")):
        sheet, idx, label = p.stem.split("__", 2)
        sheets[sheet].append((int(idx), label, p))
    f = font(18)
    tile_h = 270
    for sheet, items in sheets.items():
        items.sort()
        tiles = []
        for _, label, p in items:
            im = Image.open(p).convert("RGB")
            if im.height != tile_h:  # 拡大表示（ドットが見えるようにニアレストで）
                s = tile_h / im.height
                im = im.resize((round(im.width * s), tile_h), Image.NEAREST)
            tiles.append((label, im))
        cols = min(len(tiles), 4 if tiles[0][1].width < 300 else 3)
        rows = (len(tiles) + cols - 1) // cols
        tw = max(im.width for _, im in tiles)
        pad, lab_h = 10, 30
        sheet_im = Image.new("RGB", (cols * (tw + pad) + pad, rows * (tile_h + lab_h + pad) + pad), "white")
        d = ImageDraw.Draw(sheet_im)
        for i, (label, im) in enumerate(tiles):
            x = pad + (i % cols) * (tw + pad)
            y = pad + (i // cols) * (tile_h + lab_h + pad)
            d.text((x, y + 4), label, fill="black", font=f)
            sheet_im.paste(im, (x, y + lab_h))
        sheet_im.save(dst / f"{sheet}.png", optimize=True)
        print(dst / f"{sheet}.png")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
