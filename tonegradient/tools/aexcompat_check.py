"""ビルドした .aex を AEXCompat で（AE も管理者権限も使わずに）描画し、結果を1枚のシートに並べる。

python tools/aexcompat_check.py <AEXCompat のリポジトリ> <tonegradient.aex> <出力フォルダ> [argb8|argb16|argb32f]

- AEXCompat は先に GUI/worker をビルドしておく（broker と aex_worker が要る）。
  broker が無ければ `cargo build --manifest-path broker/Cargo.toml -p aexcompat-broker --bin broker --release`
- AEXCompat のリポジトリのパスが長いと C++ のビルドが失敗するので、短いパスに置く
- slot はエフェクトコントロールの上からの番号（グループの開始・終了も1つずつ数える）。
  パラメータの並びを変えたら下の SLOT を直す
"""
import json
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

# パラメータの番号（src/plugin.rs の params_setup の順）
SLOT = {
    "Output": 1,
    "Width": 2,
    "Texture": 3,
    "Texture Layer": 4,
    "Texture Mode": 5,
    "Amount": 6,
    "Animate Seed": 7,
    # Color Select: 8〜243（Colors 1-8 の Use 1 が 12、Color 1 が 13）
    "Offset": 245,
    "Curve": 247,
    "Curve Graph": 248,
    "Curve Power": 249,
    "Mid Amount": 253,
    "Mid Width": 254,
    "Softness": 257,
    "Texture Size": 258,
    "Antialias": 267,
    "Render": 273,
}

TEXTURES = ["Smooth", "Dot", "Line", "Bayer", "Grain", "Noise", "Layer"]


# ポップアップの選択肢の数（AEXCompat は値が選択肢の範囲内かを確かめる）
POPUP_CHOICES = {
    "Texture Mode": 2,
    "Output": 2,
    "Texture": 7,
    "Curve": 3,
    "Mid Position": 2,
    "Antialias": 4,
    "Render": 2,
}


def param(name, value, kind=None, layer=None):
    choices = []
    minimum, maximum = 0, 100000
    if kind is None:
        kind = "popup" if name in POPUP_CHOICES else "float"
    if kind == "popup":
        n = POPUP_CHOICES[name]
        choices = [str(i + 1) for i in range(n)]
        minimum, maximum = 1, n
    if name.startswith("Enable") or name == "Animate Seed":
        kind, minimum, maximum = "integer", 0, 1
    return {
        "slot": SLOT[name],
        "name": name,
        "kind": kind,
        "minimum": minimum,
        "maximum": maximum,
        "value": value,
        "choices": choices,
        "color": [0, 0, 0, 0],
        "components": [0, 0, 0],
        "component_count": 0,
        "layer_path": layer,
        "enabled": True,
        "visible": True,
        "supervised": False,
    }


def make_inputs(out):
    """初期値の Color 1 / 2（白・グレー）に合わせた入力と、テクスチャ・紙の画像"""
    w, h = 400, 300
    im = Image.new("RGB", (w, h), (255, 255, 255))
    im.paste((192, 192, 192), (0, h // 2, w, h))
    im.save(out / "input.png")
    tex = Image.new("RGB", (40, 40), (0, 0, 0))
    for y in range(40):
        for x in range(40):
            if (x // 20 + y // 20) % 2 == 0:
                tex.putpixel((x, y), (255, 255, 255))
    tex.save(out / "checker.png")


def cases(out):
    layer = str((out / "checker.png").as_posix())
    return [
        ("default", []),
        ("Dot / Softness 3px", [param("Width", 120), param("Softness", 3)]),
        ("Noise / Map", [param("Texture", 6), param("Width", 120), param("Texture Size", 8), param("Texture Mode", 2)]),
        ("Dot / Mid Color", [param("Width", 120), param("Mid Amount", 80)]),
        ("Texture Layer", [param("Texture", 7), param("Width", 800), param("Texture Layer", 0, "layer", layer)]),
        # GPU と CPU で同じ結果になるか（初期値は GPU）
        ("Dot / AA 4x4 / CPU", [param("Width", 120), param("Antialias", 3), param("Render", 2)]),
        ("Dot / AA 4x4 / GPU", [param("Width", 120), param("Antialias", 3), param("Render", 1)]),
    ]


def main(aexcompat, aex, out, pixel_format="argb8"):
    aexcompat, aex, out = Path(aexcompat).resolve(), Path(aex).resolve(), Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    broker = aexcompat / "broker" / "target" / "release" / "broker.exe"
    make_inputs(out)
    tiles = []
    for i, (label, params) in enumerate(cases(out)):
        frame_dir = out / f"case{i:02}"
        frame_dir.mkdir(exist_ok=True)
        req = {
            "schema_version": 1,
            "plugin": aex.as_posix(),
            "input_frames": [(out / "input.png").as_posix()],
            "output_directory": frame_dir.as_posix(),
            "smart": True,
            "pixel_format": pixel_format,
            "parameters": params,
        }
        req_path = out / f"case{i:02}.json"
        report = out / f"case{i:02}_report.json"
        req_path.write_text(json.dumps(req), encoding="utf-8")
        if report.exists():
            report.unlink()
        r = subprocess.run([str(broker), "render-video-batch", str(req_path), str(report)], cwd=aexcompat)
        png = frame_dir / "frame-000000.png"
        ok = r.returncode == 0 and png.exists()
        print(f"{'ok  ' if ok else 'FAIL'} {label}")
        tiles.append((label, Image.open(png).convert("RGB") if ok else Image.new("RGB", (400, 300), (255, 0, 0))))
    cols = 3
    tw, th, pad, lab = 400, 300, 10, 24
    rows = (len(tiles) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * (tw + pad) + pad, rows * (th + lab + pad) + pad), "white")
    d = ImageDraw.Draw(sheet)
    for i, (label, im) in enumerate(tiles):
        x = pad + (i % cols) * (tw + pad)
        y = pad + (i // cols) * (th + lab + pad)
        d.text((x, y + 4), label, fill="black")
        sheet.paste(im, (x, y + lab))
    sheet.save(out / "sheet.png")
    print(out / "sheet.png")


if __name__ == "__main__":
    main(*sys.argv[1:])
