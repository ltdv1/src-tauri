#!/usr/bin/env python3
"""Генератор иконки BETTERCLOUD: только логотип (облако с волнами), без фона — прозрачный PNG/ICO/ICNS."""
from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
ACCENT = (255, 91, 35)
ACCENT_2 = (255, 138, 61)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def cloud(draw, cx, cy, s, color):
    draw.ellipse([cx - 1.05 * s, cy - 0.35 * s, cx - 0.15 * s, cy + 0.55 * s], fill=color)
    draw.ellipse([cx - 0.65 * s, cy - 0.95 * s, cx + 0.55 * s, cy + 0.25 * s], fill=color)
    draw.ellipse([cx + 0.05 * s, cy - 0.45 * s, cx + 1.05 * s, cy + 0.55 * s], fill=color)
    draw.rounded_rectangle([cx - 1.05 * s, cy + 0.05 * s, cx + 1.05 * s, cy + 0.55 * s], radius=int(0.5 * s), fill=color)


img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))

# ---------- облако с вертикальным градиентом ----------
CX, CY, S = 512, 470, 215
mask = Image.new("L", (SIZE, SIZE), 0)
cloud(ImageDraw.Draw(mask), CX, CY, S, 255)

grad = Image.new("RGBA", (SIZE, SIZE))
gd = ImageDraw.Draw(grad)
for y in range(SIZE):
    t = min(1, max(0, (y - (CY - S)) / (2 * S)))
    gd.line([(0, y), (SIZE, y)], fill=lerp(ACCENT_2, ACCENT, t) + (255,))
img = Image.composite(grad, img, mask)

# мягкое свечение под логотипом (сохраняем прозрачность фона)
glow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse([CX - 380, CY - 260, CX + 380, CY + 300], fill=ACCENT + (70,))
img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(90)))
# облако поверх свечения
img = Image.composite(grad, img, mask)

# ---------- вырез: 5 баров-волн внутри облака ----------
cut = Image.new("L", (SIZE, SIZE), 0)
cd = ImageDraw.Draw(cut)
YC = CY - 10
for cx, h in [(372, 70), (432, 115), (512, 150), (592, 115), (652, 70)]:
    cd.rounded_rectangle([cx - 15, YC - h // 2, cx + 15, YC + h // 2], radius=15, fill=255)

r, g, b, a = img.split()
a = Image.composite(Image.new("L", (SIZE, SIZE), 0), a, cut)
img = Image.merge("RGBA", (r, g, b, a))

out_dir = "/home/phobos/projects/serg/bettercloud/src-tauri/icons"
sizes = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}
for name, s in sizes.items():
    img.resize((s, s), Image.LANCZOS).save(f"{out_dir}/{name}")

# ---------- .ico (несколько размеров) ----------
img.resize((256, 256), Image.LANCZOS).save(
    f"{out_dir}/icon.ico",
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)

# ---------- .icns (ручная сборка атомов) ----------
def png_atom(atype, size):
    data = img.resize((size, size), Image.LANCZOS)
    import io
    buf = io.BytesIO()
    data.save(buf, "PNG")
    payload = buf.getvalue()
    return atype.encode() + len(payload).to_bytes(4, "big") + payload


atoms = b"".join([
    png_atom("icp4", 16),    # 16x16
    png_atom("ic12", 32),    # 16x16@2x
    png_atom("icp5", 32),    # 32x32
    png_atom("ic11", 64),    # 32x32@2x
    png_atom("ic07", 128),   # 128x128
    png_atom("ic13", 256),   # 128x128@2x
    png_atom("ic08", 256),   # 256x256
    png_atom("ic09", 512),   # 512x512
    png_atom("ic10", 1024),  # 512x512@2x
])
toc_size = 8 + len(atoms)
icns = b"icns" + toc_size.to_bytes(4, "big") + atoms
with open(f"{out_dir}/icon.icns", "wb") as f:
    f.write(icns)

print("icons done (transparent logo)")
