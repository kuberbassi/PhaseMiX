"""Generate multi-size PhaseMiX Windows icon (installer/phasemix.ico)."""

from __future__ import annotations

import os
from PIL import Image, ImageDraw


def main() -> None:
    size = 512
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Obsidian dark rounded badge
    pad = 28
    draw.rounded_rectangle(
        [pad, pad, size - pad, size - pad],
        radius=116,
        fill=(20, 22, 28, 255),
        outline=(70, 78, 96, 255),
        width=10,
    )

    # Three phase-aligned equalizer/speaker bars
    bars = [
        (134, 176, 178, 344, (226, 232, 240, 255)),
        (234, 120, 278, 392, (248, 250, 252, 255)),
        (334, 198, 378, 326, (203, 213, 225, 255)),
    ]
    for x0, y0, x1, y1, color in bars:
        draw.rounded_rectangle([x0, y0, x1, y1], radius=22, fill=color)

    # Live studio status dot
    draw.ellipse([348, 114, 388, 154], fill=(34, 197, 94, 255))

    out_dir = os.path.dirname(os.path.abspath(__file__))
    ico_path = os.path.join(out_dir, "phasemix.ico")
    img.save(
        ico_path,
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print(f"Generated {ico_path}")


if __name__ == "__main__":
    main()
