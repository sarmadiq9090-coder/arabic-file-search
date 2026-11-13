#!/usr/bin/env python3
"""Render the MSIX visual assets (packaging/msix/Assets/) for the Store package.

    python3 packaging/msix/make_assets.py            # same artwork as assets/bahth-256.png
    python3 packaging/msix/make_assets.py --from-png assets/bahth-256.png

By default the icon is drawn by assets/make_icon.py, the script that made
assets/bahth-256.png and bahth.ico, at each exact size, so the 400% tiles
(up to 1240 px) stay sharp. With --from-png the given PNG is resampled
instead (fine for the small sizes, soft above 256 px).

Writes, for the scales 100/125/150/200/400:
  Square44x44Logo   (app list, taskbar)  + targetsize-16/24/32/48/256 (plated
                    and altform-unplated, for the taskbar and Explorer)
  Square150x150Logo (medium tile), Wide310x150Logo (wide tile),
  SmallTile (71x71), LargeTile (310x310), StoreLogo (50x50)
The CI workflow (.github/workflows/store.yml) indexes them into
resources.pri with makepri, which picks the right file per display scale.

Optional: --screenshots OUT frames docs/media/*.png on 1366x768 canvases
(the Store's minimum desktop screenshot size) as a stop-gap until real
full-size screenshots exist.
Requires Pillow.
"""
import argparse
import glob
import os
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(HERE, "Assets")
SCALES = [100, 125, 150, 200, 400]
TARGET_SIZES = [16, 24, 32, 48, 256]

# name: (width, height at scale 100, share of the height the icon fills)
TILES = {
    "Square44x44Logo": (44, 44, 1.0),
    "StoreLogo": (50, 50, 1.0),
    "SmallTile": (71, 71, 0.72),
    "Square150x150Logo": (150, 150, 0.62),
    "Wide310x150Logo": (310, 150, 0.62),
    "LargeTile": (310, 310, 0.56),
}


def make_renderer(png):
    if png:
        src = Image.open(png).convert("RGBA")
        return lambda px: src.resize((px, px), Image.LANCZOS)
    sys.path.insert(0, os.path.join(ROOT, "assets"))
    import make_icon  # noqa: E402  (assets/make_icon.py)

    cache = {}

    def render(px):
        if px not in cache:
            cache[px] = make_icon.render(px)
        return cache[px]

    return render


def scaled(n, scale):
    return int(n * scale / 100 + 0.5)


def tile(render, w, h, share):
    canvas = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    px = max(16, int(h * share + 0.5))
    icon = render(px)
    canvas.alpha_composite(icon, ((w - px) // 2, (h - px) // 2))
    return canvas


def save(img, name):
    img.save(os.path.join(OUT, name), optimize=True)


def assets(render):
    os.makedirs(OUT, exist_ok=True)
    for old in glob.glob(os.path.join(OUT, "*.png")):
        os.remove(old)
    count = 0
    for name, (w, h, share) in TILES.items():
        for s in SCALES:
            save(tile(render, scaled(w, s), scaled(h, s), share), f"{name}.scale-{s}.png")
            count += 1
    for t in TARGET_SIZES:
        icon = render(t)
        save(icon, f"Square44x44Logo.targetsize-{t}.png")
        save(icon, f"Square44x44Logo.targetsize-{t}_altform-unplated.png")
        count += 2
    print(f"wrote {count} files to {os.path.relpath(OUT, ROOT)}")


def screenshots(out):
    os.makedirs(out, exist_ok=True)
    for f in sorted(glob.glob(os.path.join(ROOT, "docs", "media", "*.png"))):
        shot = Image.open(f).convert("RGB")
        canvas = Image.new("RGB", (1366, 768), (32, 32, 32))
        canvas.paste(shot, ((1366 - shot.width) // 2, (768 - shot.height) // 2))
        canvas.save(os.path.join(out, os.path.basename(f)))
        print("framed", os.path.basename(f))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--from-png", help="resample this PNG instead of drawing with assets/make_icon.py")
    ap.add_argument("--screenshots", metavar="OUT", help="also frame docs/media/*.png on 1366x768 into OUT")
    args = ap.parse_args()
    assets(make_renderer(args.from_png))
    if args.screenshots:
        screenshots(args.screenshots)


if __name__ == "__main__":
    main()
