"""Draws the app icon in every size the Tauri bundle asks for. Needs Pillow."""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ACCENT = (79, 70, 229, 255)
PAPER = (255, 255, 255, 255)
SOFT = (255, 255, 255, 150)


def icon(size: int) -> Image.Image:
    scale = 4
    side = size * scale
    image = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    unit = side / 32

    draw.rounded_rectangle((unit, unit, side - unit, side - unit), radius=7 * unit, fill=ACCENT)

    # Three columns of a board; the middle one, where the work is, stands out.
    for index, (top, bottom, colour) in enumerate([(9, 19, SOFT), (9, 23, PAPER), (9, 15, SOFT)]):
        left = (7.5 + index * 6) * unit
        draw.rounded_rectangle((left, top * unit, left + 4.5 * unit, bottom * unit), radius=1.6 * unit, fill=colour)

    return image.resize((size, size), Image.LANCZOS)


def main(folder: str) -> None:
    out = Path(folder)
    out.mkdir(parents=True, exist_ok=True)
    icon(32).save(out / "32x32.png")
    icon(128).save(out / "128x128.png")
    icon(256).save(out / "128x128@2x.png")
    icon(512).save(out / "icon.png")
    icon(256).save(out / "icon.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon(1024).save(out / "icon.icns")


if __name__ == "__main__":
    main(sys.argv[1])
