#!/usr/bin/env python3
"""Checks a macOS app bundle as packaging/macos/bundle.sh lays it out, on
any system: its Info.plist, its executables, its icon and its licences.

    packaging/macos/check.py APP.app APP_ID VERSION MINIMUM_MACOS
"""

import os
import plistlib
import re
import struct
import sys

# The PNG entries an .icns holds, by type: the side of their image.
ICNS = {
    b"icp4": 16, b"ic11": 32, b"icp5": 32, b"ic12": 64, b"ic07": 128,
    b"ic13": 256, b"ic08": 256, b"ic14": 512, b"ic09": 512, b"ic10": 1024,
}
PNG = b"\x89PNG\r\n\x1a\n"
LICENSES = [
    "LICENSE", "LICENSE.rbw-ng", "LICENSE.Inter", "copyright",
    "third-party-licenses.txt", "third-party-licenses.rbw-ng.txt",
]


def icns_problems(path: str) -> list[str]:
    data = open(path, "rb").read()
    if data[:4] != b"icns" or struct.unpack(">I", data[4:8])[0] != len(data):
        return [f"{path}: not an .icns of its own length"]
    problems, found, at = [], set(), 8
    while at < len(data):
        kind, length = data[at:at + 4], struct.unpack(">I", data[at + 4:at + 8])[0]
        image = data[at + 8:at + length]
        if kind not in ICNS:
            problems.append(f"{path}: unexpected entry {kind!r}")
        elif image[:8] != PNG or image[12:16] != b"IHDR":
            problems.append(f"{path}: {kind.decode()} isn't a PNG")
        else:
            width, height = struct.unpack(">II", image[16:24])
            if (width, height) != (ICNS[kind], ICNS[kind]):
                problems.append(f"{path}: {kind.decode()} is {width}x{height}, not {ICNS[kind]}")
        found.add(kind)
        at += length
    problems += [f"{path}: no {kind.decode()} entry" for kind in ICNS if kind not in found]
    return problems


def problems(app: str, app_id: str, version: str, minimum_macos: str) -> list[str]:
    contents = os.path.join(app, "Contents")
    try:
        with open(os.path.join(contents, "Info.plist"), "rb") as file:
            info = plistlib.load(file)
    except (OSError, plistlib.InvalidFileException, ValueError) as error:
        return [f"Info.plist: {error}"]
    expected = {
        "CFBundleIdentifier": app_id,
        "CFBundlePackageType": "APPL",
        "CFBundleExecutable": "rbw-desktop",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSMinimumSystemVersion": minimum_macos,
        "NSHighResolutionCapable": True,
        "LSUIElement": False,
    }
    found = [f"Info.plist: {key} is {info.get(key)!r}, not {value!r}"
             for key, value in expected.items() if info.get(key) != value]
    for key in ["CFBundleName", "CFBundleDisplayName", "NSLocalNetworkUsageDescription"]:
        if not isinstance(info.get(key), str) or not info[key] or "@" in info[key]:
            found.append(f"Info.plist: {key} is {info.get(key)!r}")
    if len(info.get("CFBundleName", "")) > 15:
        found.append("Info.plist: CFBundleName is longer than 15 characters")
    if not re.fullmatch(r"[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+", app_id):
        found.append(f"{app_id} isn't a bundle identifier")
    for program in ["MacOS/rbw-desktop", "Helpers/rbw", "Helpers/rbw-agent"]:
        path = os.path.join(contents, program)
        if not os.path.isfile(path) or not os.access(path, os.X_OK):
            found.append(f"{program} is missing or not executable")
    icon = os.path.join(contents, "Resources", info.get("CFBundleIconFile", "") + ".icns")
    found += icns_problems(icon) if os.path.isfile(icon) else [f"{icon} is missing"]
    licenses = os.path.join(contents, "Resources", "Licenses")
    found += [f"Resources/Licenses/{name} is missing or empty" for name in LICENSES
              if not os.path.isfile(os.path.join(licenses, name))
              or not os.path.getsize(os.path.join(licenses, name))]
    return found


def main() -> None:
    if len(sys.argv) != 5:
        sys.exit(__doc__.strip().splitlines()[-1].strip())
    found = problems(*sys.argv[1:])
    for problem in found:
        print(f"{sys.argv[1]}: {problem}", file=sys.stderr)
    if found:
        sys.exit(1)


if __name__ == "__main__":
    main()
