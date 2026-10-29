#!/usr/bin/env python3
"""Generate the application icon for "بحث سريع" (assets/bahth.ico + assets/bahth-256.png).

Re-run with:  python3 assets/make_icon.py
Requires Pillow (with raqm for Arabic shaping; falls back to the isolated presentation form).
"""
import math
import os
from PIL import Image, ImageDraw, ImageFilter, ImageFont, features

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
S = 1024  # master canvas


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(len(a)))


def gradient(size, c0, c1):
    """Diagonal gradient from top-left (c0) to bottom-right (c1)."""
    small = 256
    g = Image.new("RGB", (small, small))
    px = g.load()
    for y in range(small):
        for x in range(small):
            t = (x + y) / (2 * (small - 1))
            t = t * t * (3 - 2 * t)  # smoothstep
            px[x, y] = lerp(c0, c1, t)
    return g.resize((size, size), Image.BICUBIC)


def rounded_mask(size, radius, inset=0):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle((inset, inset, size - 1 - inset, size - 1 - inset), radius=radius, fill=255)
    return m


def tile(size, simple=False):
    """Background tile: rounded square, blue->teal gradient, soft top highlight."""
    inset = int(size * 0.03)
    radius = int(size * 0.22)
    bg = gradient(size, (37, 99, 235), (13, 182, 170)).convert("RGBA")  # blue-600 -> teal
    if not simple:
        # gentle glossy highlight on the upper half
        hl = Image.new("L", (size, size), 0)
        d = ImageDraw.Draw(hl)
        d.ellipse((-size * 0.4, -size * 0.95, size * 1.4, size * 0.40), fill=34)
        hl = hl.filter(ImageFilter.GaussianBlur(size * 0.12))
        white = Image.new("RGBA", (size, size), (255, 255, 255, 255))
        bg = Image.composite(white, bg, hl)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(bg, (0, 0), rounded_mask(size, radius, inset))
    return out


def glass_geometry(size):
    cx, cy = size * 0.42, size * 0.42  # lens centre
    r = size * 0.215  # lens radius (to centre of ring)
    ring = size * 0.07  # ring thickness
    return cx, cy, r, ring


def draw_glass(img, with_letter=True, ring_scale=1.0, shadow=True):
    size = img.size[0]
    cx, cy, r, ring = glass_geometry(size)
    ring *= ring_scale
    ang = math.radians(45)
    h0 = r + ring * 0.35
    h1 = size * 0.22
    hx0, hy0 = cx + math.cos(ang) * h0, cy + math.sin(ang) * h0
    hx1, hy1 = cx + math.cos(ang) * (h0 + h1), cy + math.sin(ang) * (h0 + h1)
    hw = ring * 1.55

    def shape(draw, fill):
        draw.ellipse((cx - r - ring / 2, cy - r - ring / 2, cx + r + ring / 2, cy + r + ring / 2), outline=fill, width=int(ring))
        draw.line((hx0, hy0, hx1, hy1), fill=fill, width=int(hw))
        cr = hw / 2  # round handle end
        draw.ellipse((hx1 - cr, hy1 - cr, hx1 + cr, hy1 + cr), fill=fill)

    if shadow:
        sh = Image.new("L", img.size, 0)
        shape(ImageDraw.Draw(sh), 255)
        sh = sh.filter(ImageFilter.GaussianBlur(size * 0.025))
        shadow_img = Image.new("RGBA", img.size, (5, 30, 60, 0))
        shadow_img.putalpha(sh.point(lambda v: v * 0.45))
        off = Image.new("RGBA", img.size, (0, 0, 0, 0))
        off.paste(shadow_img, (int(size * 0.012), int(size * 0.025)))
        # clip shadow to tile
        a = Image.composite(off, Image.new("RGBA", img.size, (0, 0, 0, 0)), img.split()[3])
        img.alpha_composite(a)

    # lens glass: faint white tint
    lens = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(lens).ellipse((cx - r, cy - r, cx + r, cy + r), fill=(255, 255, 255, 40))
    img.alpha_composite(lens)

    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    shape(ImageDraw.Draw(layer), (255, 255, 255, 255))
    img.alpha_composite(layer)

    if with_letter:
        draw_letter(img, cx, cy, r)


def draw_letter(img, cx, cy, r):
    size = img.size[0]
    fsize = int(r * 2.0)
    font = ImageFont.truetype(FONT, fsize)
    if features.check("raqm"):
        text, kw = "ب", {"direction": "rtl", "language": "ar", "features": ["-init", "-medi", "-fina"]}
    else:
        text, kw = "ﺏ", {}  # ARABIC LETTER BEH ISOLATED FORM
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    l, t, rr, b = d.textbbox((0, 0), text, font=font, **kw)
    # scale the glyph so its ink box fits comfortably inside the lens
    fit = min((r * 1.25) / (rr - l), (r * 1.1) / (b - t))
    font = ImageFont.truetype(FONT, max(8, int(fsize * fit)))
    l, t, rr, b = d.textbbox((0, 0), text, font=font, **kw)
    w, h = rr - l, b - t
    # optical centring: nudge up slightly since the dot hangs below
    x = cx - w / 2 - l
    y = cy - h / 2 - t
    d.text((x, y), text, font=font, fill=(255, 255, 255, 255), **kw)
    img.alpha_composite(layer)


def render(px):
    """Render the icon at `px` pixels. Small sizes get a simplified design."""
    if px <= 16:
        detail, letter, ring_scale = "simple", False, 1.45
    elif px <= 24:
        detail, letter, ring_scale = "simple", False, 1.3
    elif px <= 32:
        detail, letter, ring_scale = "simple", False, 1.15
    else:
        detail, letter, ring_scale = "full", True, 1.0
    ss = S if px >= 64 else max(px * 16, 256)
    img = tile(ss, simple=(detail == "simple"))
    draw_glass(img, with_letter=letter, ring_scale=ring_scale, shadow=(detail == "full"))
    out = img.resize((px, px), Image.LANCZOS)
    if px <= 32:
        # firm up edges a touch for crispness on small sizes
        out = out.filter(ImageFilter.UnsharpMask(radius=0.6, percent=60, threshold=0))
    return out


def main():
    sizes = [16, 24, 32, 48, 64, 128, 256]
    imgs = [render(s) for s in sizes]
    imgs[-1].save(os.path.join(HERE, "bahth-256.png"))
    # preview sheet of all sizes (not shipped)
    sheet = Image.new("RGBA", (sum(sizes) + 10 * len(sizes) + 10, 276), (240, 240, 240, 255))
    x = 10
    for s, im in zip(sizes, imgs):
        sheet.alpha_composite(im, (x, 10))
        x += s + 10
    sheet.save(os.path.join(HERE, "preview-sizes.png"))
    # ICO: Pillow writes each size from the matching image in append_images
    imgs[-1].save(os.path.join(HERE, "bahth.ico"), format="ICO",
                  sizes=[(s, s) for s in sizes], append_images=imgs[:-1])
    print("wrote bahth.ico, bahth-256.png")


if __name__ == "__main__":
    main()
