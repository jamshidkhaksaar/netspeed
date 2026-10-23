#!/usr/bin/env python3
"""Render NetSpeed's original SVG into Windows branding assets using Pillow.

The intentionally small SVG vocabulary (rounded rectangles and polygons) keeps
its vector source authoritative without a browser, external renderer, or image
service. Each ICO frame is rendered independently, not reduced from another ICO
frame. Run with Python 3 and Pillow; no network access is required.

    python -B scripts/generate_icon.py
    python -B scripts/generate_icon.py --check

Only assets/netspeed.png, assets/netspeed-16.png and assets/netspeed.ico are
written. --check verifies that those committed assets match the vector source.
"""

from __future__ import annotations

import argparse
from io import BytesIO
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image, ImageColor, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
SOURCE = ASSETS / "netspeed.svg"
ICON_SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
PNG_SIZE = 1024
SVG_NS = "{http://www.w3.org/2000/svg}"


def render_icon(svg: ET.Element, size: int) -> Image.Image:
    """Rasterize the supported flat vector shapes with supersampled edges."""
    if svg.tag != SVG_NS + "svg" or svg.get("viewBox") != "0 0 256 256":
        raise ValueError("Expected a NetSpeed SVG with viewBox='0 0 256 256'")

    supersampling = 8 if size <= 256 else 4
    scale = size * supersampling / 256
    image = Image.new("RGBA", (size * supersampling,) * 2, (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    for element in svg:
        tag = element.tag.removeprefix(SVG_NS)
        if tag in ("title", "desc", "metadata"):
            continue
        if element.get("transform") or element.get("stroke"):
            raise ValueError("Transforms and strokes need an explicit renderer update")
        fill = ImageColor.getcolor(element.attrib["fill"], "RGBA")
        if tag == "rect":
            x = float(element.get("x", "0")) * scale
            y = float(element.get("y", "0")) * scale
            width = float(element.attrib["width"]) * scale
            height = float(element.attrib["height"]) * scale
            radius = float(element.get("rx", "0")) * scale
            # Pillow's end coordinates are inclusive; SVG dimensions are not.
            draw.rounded_rectangle(
                (x, y, x + width - 1, y + height - 1), radius, fill=fill
            )
        elif tag == "polygon":
            coordinates = element.attrib["points"].replace(",", " ").split()
            if len(coordinates) < 6 or len(coordinates) % 2:
                raise ValueError("SVG polygon must contain at least three x/y pairs")
            points = [
                (float(coordinates[i]) * scale, float(coordinates[i + 1]) * scale)
                for i in range(0, len(coordinates), 2)
            ]
            draw.polygon(points, fill=fill)
        else:
            raise ValueError(f"Unsupported SVG element: {tag}")

    image = image.resize((size, size), Image.Resampling.LANCZOS)
    # Lanczos can leave one-unit alpha ringing outside an otherwise empty
    # corner. Trim only that invisible halo, preserving antialiased edges.
    image.putalpha(image.getchannel("A").point(lambda alpha: 0 if alpha < 3 else alpha))
    return image


def png_bytes(image: Image.Image) -> bytes:
    output = BytesIO()
    image.save(output, "PNG", compress_level=9)
    return output.getvalue()


def ico_bytes(frames: dict[int, Image.Image]) -> bytes:
    output = BytesIO()
    frames[256].save(
        output,
        "ICO",
        sizes=[(size, size) for size in ICON_SIZES],
        append_images=[frames[size] for size in ICON_SIZES if size != 256],
    )
    return output.getvalue()


def verify_icon(data: bytes, frames: dict[int, Image.Image]) -> None:
    """Decode every embedded resolution and verify its independent RGBA image."""
    with Image.open(BytesIO(data)) as icon:
        expected_sizes = {(size, size) for size in ICON_SIZES}
        if icon.info.get("sizes") != expected_sizes:
            raise ValueError(f"ICO does not contain the exact required sizes: {icon.info}")
        for size, expected in frames.items():
            decoded = icon.ico.getimage((size, size)).convert("RGBA")
            if decoded.tobytes() != expected.tobytes():
                raise ValueError(f"ICO {size}px frame differs from the vector render")
            if decoded.getpixel((0, 0))[3] != 0:
                raise ValueError(f"ICO {size}px frame lost its transparent outer corner")
            if decoded.getpixel((size // 2, size // 2))[3] != 255:
                raise ValueError(f"ICO {size}px frame lost its opaque badge")

    # At the smallest native tray resolution, each directional shaft still has
    # a solid-color core. Direction as well as color distinguishes down/up.
    small = frames[16]
    down = small.getpixel((4, 5))
    up = small.getpixel((11, 10))
    if not (down[1] > down[0] and down[1] > down[2] and down[3] == 255):
        raise ValueError("The 16px download shaft is not a clear, opaque green")
    if not (up[2] > up[0] and up[1] > up[0] and up[3] == 255):
        raise ValueError("The 16px upload shaft is not a clear, opaque cyan")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check", action="store_true", help="verify committed assets without writing files"
    )
    args = parser.parse_args()
    svg = ET.parse(SOURCE).getroot()
    frames = {size: render_icon(svg, size) for size in ICON_SIZES}
    icon = ico_bytes(frames)
    verify_icon(icon, frames)
    outputs = {
        ASSETS / "netspeed.png": png_bytes(render_icon(svg, PNG_SIZE)),
        ASSETS / "netspeed-16.png": png_bytes(frames[16]),
        ASSETS / "netspeed.ico": icon,
    }
    for path, data in outputs.items():
        if args.check:
            if not path.is_file() or path.read_bytes() != data:
                raise SystemExit(f"Out-of-date or missing icon asset: {path}")
            print(f"Verified {path.relative_to(ROOT)} ({len(data):,} bytes)")
        else:
            path.write_bytes(data)
            print(f"Generated {path.relative_to(ROOT)} ({len(data):,} bytes)")
    print("Verified RGBA ICO frames: " + ", ".join(f"{size}px" for size in ICON_SIZES))


if __name__ == "__main__":
    main()
