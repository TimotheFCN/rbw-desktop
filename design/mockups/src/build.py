#!/usr/bin/env python3
"""Builds the desktop mockup pages next to this directory.

Each page is one self-contained HTML file: the shared stylesheet, the icon
sprite, and one or more mocked windows on a wallpaper.
"""
import pathlib

HERE = pathlib.Path(__file__).parent
CSS = (HERE / "base.css").read_text()
SPRITE = (HERE / "sprite.html").read_text()


def ic(name, extra=""):
    return f'<svg class="ic" {extra}><use href="#i-{name}"/></svg>'


def tile(letter, hue, size=""):
    return f'<span class="tile {size}" style="--h:{hue}">{letter}</span>'


def ring(fraction=0.6):
    dash = 2 * 3.14159 * 6
    return (
        '<svg class="ring" viewBox="0 0 16 16"><circle class="t" cx="8" cy="8" r="6"/>'
        f'<circle class="p" cx="8" cy="8" r="6" stroke-dasharray="{dash * fraction:.1f} {dash:.1f}"/></svg>'
    )


PASSWORD = "vT<i>7</i><u>#</u>kq<i>2</i>Lw<u>!</u>m<i>9</i>Rz<u>@</u>eB<i>4</i>xN<u>^</u>pd"

ITEMS = [
    ("Airbnb", "alex@example.org", "A", 350),
    ("Amazon", "alex@example.org", "A", 32),
    ("Booking.com", "alex@example.org", "B", 215),
    ("Disney+", "family@example.org", "D", 240),
    ("Google", "alex.martin@gmail.com", "G", 135),
    ("Netflix", "family@example.org", "N", 0),
    ("PayPal", "alex@example.org", "P", 205),
    ("Spotify", "alex@example.org", "S", 145),
]


def sidebar(gnome):
    top = "Vault" if gnome else '<span class="lights"><i></i><i></i><i></i></span>'
    rows = [
        ("grid", "All Items", True),
        ("star", "Favorites", False),
        (None, "Types", None),
        ("key", "Logins", False),
        ("card", "Cards", False),
        ("user", "Identities", False),
        ("note", "Notes", False),
        (None, "Folders", None),
        ("folder", "Personal", False),
        ("folder", "Family", False),
        ("folder", "Work", False),
    ]
    out = [f'<div class="side"><div class="top">{top}</div>']
    for icon, label, on in rows:
        if icon is None:
            out.append(f'<div class="lab">{label}</div>')
        else:
            out.append(f'<div class="nav{" on" if on else ""}">{ic(icon)}{label}</div>')
    out.append(
        '<div class="me"><span class="ava">A</span>'
        '<span class="grow tr" style="font-weight:500">Alex</span>'
        f'<span class="dim" style="display:inline-flex">{ic("lock")}</span></div></div>'
    )
    return "".join(out)


def item_list(gnome):
    plus = (
        f'<span class="pill round flat">{ic("plus")}</span>'
        if gnome
        else f'<span class="pill round">{ic("plus")}</span>'
    )
    out = [
        '<div class="list"><div class="bar">'
        f'<span class="pill search grow">{ic("search")}Search</span>{plus}</div>'
    ]
    for name, user, letter, hue in ITEMS:
        on = " on" if name == "Amazon" else ""
        glyph = letter or ""
        out.append(
            f'<div class="li{on}">{tile(glyph, hue)}'
            f'<div class="grow"><div class="n tr">{name}</div><div class="u tr">{user}</div></div></div>'
        )
    out.append("</div>")
    return "".join(out)


def detail(gnome):
    if gnome:
        bar = (
            '<div class="bar"><span class="grow"></span>'
            f'<span class="pill round flat">{ic("star")}</span><span class="pill">Edit</span>'
            f'<span class="wc" style="margin-left:6px">{ic("x")}</span></div>'
        )
    else:
        bar = (
            '<div class="bar"><span class="grow"></span>'
            f'<span class="pill round">{ic("star")}</span><span class="pill">Edit</span></div>'
        )
    return f"""<div class="det">{bar}<div class="body">
  <div style="display:flex;align-items:center;gap:13px;margin-bottom:14px">
    {tile("A", 32, "lg")}
    <div class="grow"><div class="tr" style="font-size:20px;font-weight:700;letter-spacing:-.01em">Amazon</div><div class="dim">Login</div></div>
  </div>
  <div class="card">
    <div class="cr"><div class="grow"><span class="l">Username</span><div class="tr">alex@example.org</div></div><span class="ok" style="display:inline-flex;align-items:center;gap:4px;font-size:12px;font-weight:500">{ic("check", 'style="color:inherit"')}Copied</span></div>
    <div class="cr"><div class="grow"><span class="l">Password</span><div style="letter-spacing:.16em">••••••••••••</div></div>{ic("eye")}{ic("copy")}</div>
    <div class="cr"><div class="grow"><span class="l">Verification code</span><div style="display:flex;align-items:center;gap:7px;font-size:15px;font-weight:600;font-variant-numeric:tabular-nums">492 113 {ring(0.6)}</div></div>{ic("copy")}</div>
  </div>
  <div class="card" style="margin-top:10px">
    <div class="cr"><div class="grow"><span class="l">Website</span><div class="tr" style="color:var(--accent)">amazon.com</div></div>{ic("ext")}</div>
  </div>
  <div class="card" style="margin-top:10px">
    <div class="cr"><div class="grow"><span class="l">Notes</span><div>Shared with the family. Prime renews in March.</div></div></div>
  </div>
</div></div>"""


def new_item_sheet():
    return f"""<div class="scrim"></div>
<div class="sheet">
  <div style="display:flex;align-items:center;margin-bottom:12px"><b style="font-size:15px" class="grow">New Item</b></div>
  <div class="seg"><span class="on">Login</span><span>Card</span><span>Identity</span><span>Note</span></div>
  <div class="card" style="margin-top:12px">
    <div class="fr"><span class="l">Name</span><span class="grow tr">Netflix</span></div>
    <div class="fr"><span class="l">Username</span><span class="grow tr">family@example.org</span></div>
    <div class="fr"><span class="l">Password</span><span class="grow tr mono pw" style="font-size:12px">{PASSWORD}</span><span class="pill round" style="width:26px;height:26px">{ic("sync")}</span></div>
    <div class="fr"><span class="l">Website</span><span class="grow tr">netflix.com<span class="caret"></span></span></div>
  </div>
  <div style="display:flex;align-items:center;gap:6px;margin:8px 13px 0;font-size:12px" class="dim"><span style="width:7px;height:7px;border-radius:50%;background:var(--ok)"></span>Strong password, generated for you</div>
  <div class="card" style="margin-top:12px">
    <div class="fr"><span class="l">Folder</span><span class="grow">Family</span>{ic("updown", 'style="color:var(--faint)"')}</div>
    <div class="fr"><span class="grow">Add to Favorites</span><span class="sw off"></span></div>
  </div>
  <div style="display:flex;gap:8px;margin-top:14px"><span class="grow"></span><span class="pill">Cancel</span><span class="pill acc" style="padding:0 18px">Save</span></div>
</div>"""


def vault(theme, gnome=False, sheet=False):
    cls = "win vault t-light adw" if gnome else f"win vault t-{theme}"
    return (
        f'<div class="{cls}">{sidebar(gnome)}{item_list(gnome)}{detail(gnome)}'
        f'{new_item_sheet() if sheet else ""}</div>'
    )


def popover_footer():
    return (
        '<div style="display:flex;gap:8px;margin-top:12px">'
        f'<span class="pill grow">Open Vault</span><span class="pill round">{ic("lock")}</span></div>'
    )


def popover_items():
    rows = []
    for i, (name, user, letter, hue) in enumerate([ITEMS[1], ITEMS[5], ITEMS[4], ITEMS[6]]):
        hov = " hov" if i == 0 else ""
        action = '<span class="pill" style="height:26px;font-size:12px">Copy</span>' if i == 0 else ""
        rows.append(
            f'<div class="li{hov}">{tile(letter, hue, "sm")}'
            f'<div class="grow"><div class="n tr">{name}</div><div class="u tr">{user}</div></div>{action}</div>'
        )
    return f"""<div class="win pop t-light">
  <div class="seg"><span class="on">Passwords</span><span>Generator</span></div>
  <div class="pill search" style="display:flex;margin-top:10px">{ic("search")}Search</div>
  <div class="plab">Recently used</div>
  {"".join(rows)}
  {popover_footer()}
</div>"""


def popover_generator():
    return f"""<div class="win pop t-light">
  <div class="seg"><span>Passwords</span><span class="on">Generator</span></div>
  <div class="card" style="margin-top:10px;padding:16px 12px;text-align:center">
    <div class="mono pw" style="font-size:16px;font-weight:500;word-break:break-all;line-height:1.5">{PASSWORD}</div>
    <div class="dim" style="display:inline-flex;align-items:center;gap:6px;margin-top:8px;font-size:12px"><span style="width:7px;height:7px;border-radius:50%;background:var(--ok)"></span>Strong</div>
  </div>
  <div style="display:flex;gap:8px;margin-top:10px"><span class="pill acc grow">Copy</span><span class="pill round">{ic("sync")}</span></div>
  <div class="card" style="margin-top:10px">
    <div class="fr" style="padding:6px"><div class="seg grow"><span class="on">Password</span><span>Passphrase</span></div></div>
    <div class="fr"><span>Length</span><span class="track" style="margin:0 4px 0 6px"><i style="width:45%"></i><b style="left:45%"></b></span><b style="font-variant-numeric:tabular-nums">20</b></div>
    <div class="fr"><span class="grow">Numbers</span><span class="sw"></span></div>
    <div class="fr"><span class="grow">Symbols</span><span class="sw"></span></div>
  </div>
  {popover_footer()}
</div>"""


def menu_bar():
    return (
        '<div class="mbar">'
        '<span class="me-on"><svg><use href="#i-key"/></svg></span><span>Tue 24 Nov&nbsp;&nbsp;10:42</span></div>'
    )


def lock_screen():
    return f"""<div class="win lock t-light">
  <span class="lights"><i></i><i></i><i></i></span>
  <span class="appic">{ic("lock")}</span>
  <div style="font-size:20px;font-weight:700;margin-top:16px;letter-spacing:-.01em">Welcome back</div>
  <div class="dim" style="margin-top:3px">alex@example.org</div>
  <div class="pill" style="width:270px;height:38px;margin-top:20px;padding:0 5px 0 16px;justify-content:flex-start;background:var(--field)">
    <span class="grow dim" style="text-align:left;font-weight:400">Master password</span>
    <span class="pill round acc" style="width:28px;height:28px">{ic("arrow")}</span>
  </div>
  <div style="display:inline-flex;align-items:center;gap:6px;margin-top:16px;color:var(--accent);font-weight:500">{ic("finger")}Unlock with Touch ID</div>
</div>"""


PAGES = {
    "01-vault-light": f'<div class="stage nowrap wp-light">{vault("light")}</div>',
    "02-vault-dark-new-item": f'<div class="stage nowrap wp-dark">{vault("dark", sheet=True)}</div>',
    "03-menu-bar": f'<div class="stage wp-light" style="padding-top:44px">{menu_bar()}{popover_items()}{popover_generator()}</div>',
    "04-lock-screen": f'<div class="stage wp-light">{lock_screen()}</div>',
    "05-linux-gnome": f'<div class="stage nowrap" style="background:linear-gradient(160deg,#2c5aa0,#77459a)">{vault("light", gnome=True)}</div>',
}

HEAD = '<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">\n'

for name, body in PAGES.items():
    out = HERE.parent / f"{name}.html"
    out.write_text(f"{HEAD}<style>\n{CSS}</style>\n{SPRITE}{body}\n")
    print(out.name)
