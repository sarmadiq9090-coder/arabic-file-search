#!/usr/bin/env python3
"""Copy the InstaFind icon (FastKiwi brand v2) into assets/.

The art is drawn and exported in the site repository, sarmadiq9090-coder.github.io, folder
brand/ (brand/source/gen.py and export.py). The masters are copied here in assets/brand/.
This script takes that repository's brand/icons/instafind folder and writes:

  bahth.ico             application icon, resource 1 (16-256 px, hand-tuned 16/20/24/32 px)
  bahth-256.png         README, package-manager manifests
  bahth-tray-dark.ico   tray symbol without the tile for a dark taskbar, resource 2
  bahth-tray-light.ico  the same in the darker ink tone for a light taskbar, resource 3

Re-run with:  python3 assets/make_icon.py <site-repo>/brand/icons/instafind
Requires Pillow.
"""
import os
import shutil
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
TRAY_SIZES = [16, 20, 24, 32, 40, 48, 64]  # SM_CXSMICON from 100 % to 400 % scaling


def tray_ico(msix, suffix, out):
    images = [Image.open(os.path.join(msix, f"Square44x44Logo.targetsize-{s}{suffix}.png")).convert("RGBA") for s in TRAY_SIZES]
    images[-1].save(out, format="ICO", sizes=[(s, s) for s in TRAY_SIZES], append_images=images[:-1])


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    shutil.copyfile(os.path.join(src, "instafind.ico"), os.path.join(HERE, "bahth.ico"))
    shutil.copyfile(os.path.join(src, "png", "instafind-256.png"), os.path.join(HERE, "bahth-256.png"))
    tray_ico(os.path.join(src, "msix"), "_altform-unplated", os.path.join(HERE, "bahth-tray-dark.ico"))
    tray_ico(os.path.join(src, "msix"), "_altform-lightunplated", os.path.join(HERE, "bahth-tray-light.ico"))
    os.makedirs(os.path.join(HERE, "brand"), exist_ok=True)
    for name in sorted(os.listdir(src)):
        if name.endswith(".svg"):
            shutil.copyfile(os.path.join(src, name), os.path.join(HERE, "brand", name))
    print("wrote bahth.ico, bahth-256.png, bahth-tray-dark.ico, bahth-tray-light.ico, brand/*.svg")


if __name__ == "__main__":
    main()
