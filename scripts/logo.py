"""Draw the Satchel logo and render every icon the apps ship.

Usage: python3 scripts/logo.py

Writes the SVG sources (see docs/decisions/0020-logo.md), then renders the
Tauri icon set into apps/desktop/icons with the Tauri CLI from ui/
(run `npm install` there first). The 16–32px slots of icon.icns, icon.ico and
32x32.png use the simplified drawing, which stays legible that small.
Needs macOS for iconutil.
"""

import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "apps/desktop/icons"

GRAD_TOP, GRAD_BOT = "#d4925a", "#b06f3c"
FG, SHADE, CUT = "#fffaf4", "#f3dcc4", "#9c5a28"

# The bag in a 0–100 box: body top Y0, height H, flap point D below the top.
Y0, H, D = 36, 48, 32
B = Y0 + H
GLYPH_CY = 52.5  # vertical centre of the ink, handle top to body bottom


def glyph(small: bool, fg=FG, shade=SHADE, cut=CUT) -> str:
    """The satchel. Small drops the stitching and clasp dot and thickens the handle."""
    body = f'<rect x="14" y="{Y0}" width="72" height="{H}" rx="{10 if small else 8}"/>'
    out = [
        f'<path d="M37 {Y0+1} V{Y0-5} Q37 {Y0-15} 50 {Y0-15} Q63 {Y0-15} 63 {Y0-5} V{Y0+1}" '
        f'fill="none" stroke="{fg}" stroke-width="{7.5 if small else 5.5}" stroke-linecap="round"/>',
        f'<defs><clipPath id="bag">{body}</clipPath></defs>',
        body.replace("/>", f' fill="{fg}"/>'),
    ]
    top = Y0 + 14
    for x in (27, 63):
        out.append(f'<rect x="{x}" y="{top}" width="10" height="{B-4-top}" rx="2.5" fill="{shade}"/>')
    # The flap's edge starts outside the bag, along its own slope, and is clipped at the sides:
    # a line ending exactly at the side would leave a notch where its slanted end meets the edge.
    slope = (D - 2.5 - 11) / (45.5 - 14)
    ey = Y0 + 11 - 6 * slope
    out += [
        f'<path d="M14 {Y0+8} Q14 {Y0} 22 {Y0} H78 Q86 {Y0} 86 {Y0+8} V{Y0+11} L54.5 {Y0+D-2.5} '
        f'Q50 {Y0+D+0.8} 45.5 {Y0+D-2.5} L14 {Y0+11} Z" fill="{shade}"/>',
        f'<path d="M8 {ey:.2f} L45.5 {Y0+D-2.5} Q50 {Y0+D+0.8} 54.5 {Y0+D-2.5} L92 {ey:.2f}" fill="none" '
        f'stroke="{cut}" stroke-width="{3.6 if small else 3.2}" stroke-linejoin="round" clip-path="url(#bag)"/>',
    ]
    if not small:
        out.append(
            f'<path d="M18.5 {Y0+16} V{B-8.5} Q18.5 {B-4} 23 {B-4} H77 Q81.5 {B-4} 81.5 {B-8.5} V{Y0+16}" '
            f'fill="none" stroke="{cut}" stroke-width="1.4" stroke-dasharray="2.6 2.1" stroke-linecap="round" opacity="0.7"/>'
        )
    out.append(f'<circle cx="50" cy="{Y0+D}" r="{6.4 if small else 5.8}" fill="{cut}"/>')
    if not small:
        out.append(f'<circle cx="50" cy="{Y0+D}" r="2.2" fill="{fg}"/>')
    return "\n    ".join(out)


def tile_svg(small: bool, macos_grid: bool) -> str:
    """The bag on the leather tile: on the macOS 824/1024 grid with a shadow, or full bleed."""
    tx, tw, rx = (9.77, 80.47, 18.1) if macos_grid else (0, 100, 22)
    scale = (0.76 if small else 0.70) * tw / 92
    gx, gy = 50 - 50 * scale, 50 - GLYPH_CY * scale
    filt = shadow = ""
    if macos_grid:
        filt = ('<filter id="shadow" x="-10%" y="-10%" width="120%" height="125%">'
                '<feDropShadow dx="0" dy="0.8" stdDeviation="0.9" flood-color="#000" flood-opacity="0.25"/></filter>')
        shadow = ' filter="url(#shadow)"'
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="1024" height="1024">
  <defs>
    <linearGradient id="leather" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{GRAD_TOP}"/>
      <stop offset="1" stop-color="{GRAD_BOT}"/>
    </linearGradient>
    {filt}
  </defs>
  <rect x="{tx}" y="{tx}" width="{tw}" height="{tw}" rx="{rx}" fill="url(#leather)"{shadow}/>
  <g transform="translate({gx:.3f} {gy:.3f}) scale({scale:.4f})">
    {glyph(small)}
  </g>
</svg>
'''


def mark_svg() -> str:
    """The bag alone on a transparent background, for the website and documents."""
    body = glyph(False, fg=GRAD_BOT, shade=GRAD_TOP, cut=FG).replace("\n    ", "\n  ")
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="12 18 76 68" width="760" height="680">
  {body}
</svg>
'''


def render(svg: Path, out: Path, sizes: list[int] | None = None) -> None:
    """Render with `tauri icon`: the full set, or just the given PNG sizes."""
    cmd = ["npx", "tauri", "icon", str(svg), "-o", str(out)]
    if sizes:
        cmd += ["-p", ",".join(map(str, sizes))]
    done = subprocess.run(cmd, cwd=ROOT / "ui", capture_output=True, text=True)
    if done.returncode:
        raise SystemExit(f"tauri icon failed:\n{done.stderr}")


def write_ico(path: Path, entries: list[tuple[int, Path]]) -> None:
    """An .ico of PNG-compressed images (Vista and later)."""
    data = [p.read_bytes() for _, p in entries]
    head = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    for (size, _), d in zip(entries, data):
        head += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(d), offset)
        offset += len(d)
    path.write_bytes(head + b"".join(data))


def main() -> None:
    sources = {
        ICONS / "icon.svg": tile_svg(small=False, macos_grid=True),
        ICONS / "icon-small.svg": tile_svg(small=True, macos_grid=True),
        ROOT / "ui/public/favicon.svg": tile_svg(small=True, macos_grid=False),
        ROOT / "docs/brand/satchel-mark.svg": mark_svg(),
    }
    for path, text in sources.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        full, det, small, fav = tmp / "full", tmp / "det", tmp / "small", tmp / "fav"
        render(ICONS / "icon.svg", full)
        render(ICONS / "icon.svg", det, [48, 64, 128, 256, 512, 1024])
        render(ICONS / "icon-small.svg", small, [16, 30, 32])
        render(ROOT / "ui/public/favicon.svg", fav, [16, 24, 32])

        for png in full.glob("*.png"):
            shutil.copy(png, ICONS / png.name)
        shutil.copy(small / "32x32.png", ICONS / "32x32.png")
        shutil.copy(small / "30x30.png", ICONS / "Square30x30Logo.png")
        shutil.copy(det / "1024x1024.png", ICONS / "icon-source.png")

        iconset = tmp / "Satchel.iconset"
        iconset.mkdir()
        slots = {
            "icon_16x16": small / "16x16.png", "icon_16x16@2x": small / "32x32.png",
            "icon_32x32": small / "32x32.png", "icon_32x32@2x": det / "64x64.png",
            "icon_128x128": det / "128x128.png", "icon_128x128@2x": det / "256x256.png",
            "icon_256x256": det / "256x256.png", "icon_256x256@2x": det / "512x512.png",
            "icon_512x512": det / "512x512.png", "icon_512x512@2x": det / "1024x1024.png",
        }
        for name, src in slots.items():
            shutil.copy(src, iconset / f"{name}.png")
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS / "icon.icns")], check=True)

        write_ico(ICONS / "icon.ico", [
            (16, fav / "16x16.png"), (24, fav / "24x24.png"), (32, fav / "32x32.png"),
            (48, det / "48x48.png"), (64, det / "64x64.png"), (256, det / "256x256.png"),
        ])
    print(f"Wrote the logo sources and the icon set in {ICONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
