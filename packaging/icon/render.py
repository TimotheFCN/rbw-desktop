#!/usr/bin/env python3
"""Renders the app icon at every size the packages use: the PNGs of the
Linux icon theme in hicolor/ and macOS's AppIcon.icns, from icon.svg and, up
to 32 pixels, icon-small.svg. Needs rsvg-convert (librsvg). Run it after
changing either drawing and commit what it writes; the packages take the
scalable icon straight from icon.svg.

    packaging/icon/render.py
"""

import pathlib
import struct
import subprocess
import tempfile

APP_ID = "io.github.timothefcn.RbwDesktop"
HERE = pathlib.Path(__file__).resolve().parent
LINUX_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
# The PNG entries of an .icns, by type and side in pixels: each point size
# at 1x and at 2x (the types iconutil writes).
ICNS = [
    (b"icp4", 16), (b"ic11", 32),
    (b"icp5", 32), (b"ic12", 64),
    (b"ic07", 128), (b"ic13", 256),
    (b"ic08", 256), (b"ic14", 512),
    (b"ic09", 512), (b"ic10", 1024),
]


def render(side: int, out: pathlib.Path) -> bytes:
    source = HERE / ("icon-small.svg" if side <= 32 else "icon.svg")
    subprocess.run(
        ["rsvg-convert", "--width", str(side), "--height", str(side),
         "--output", str(out), str(source)],
        check=True,
    )
    return out.read_bytes()


def icns(entries: list[tuple[bytes, bytes]]) -> bytes:
    """An .icns file: a header, then each entry's type, length and data."""
    body = b"".join(kind + struct.pack(">I", 8 + len(data)) + data for kind, data in entries)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main() -> None:
    for side in LINUX_SIZES:
        out = HERE / "hicolor" / f"{side}x{side}" / "apps" / f"{APP_ID}.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        render(side, out)
    with tempfile.TemporaryDirectory() as scratch:
        pngs = {side: render(side, pathlib.Path(scratch) / f"{side}.png")
                for side in sorted({side for _, side in ICNS})}
    (HERE / "AppIcon.icns").write_bytes(icns([(kind, pngs[side]) for kind, side in ICNS]))


if __name__ == "__main__":
    main()
