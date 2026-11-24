# Development

## Crates

| Crate | Package | What it holds |
| --- | --- | --- |
| `crates/core` | `rbw-desktop-core` | The `Backend` trait over rbw and `FakeBackend` with demo vaults. No GPUI. |
| `crates/ui` | `rbw-desktop-ui` | The design system: theme tokens, motion, window chrome, glass, components. Its `gallery` example shows every component. |
| `crates/app` | `rbw-desktop` | The app. One module per screen (`vault/`), plus `backend.rs` and `clipboard.rs`. |

GPUI comes from `gpui-kit` with its default features off: GPUI itself (the
`gpui-pre` snapshots of Zed's GPUI) and `gpui-base`, the unstyled behaviour
layer (activation, focus, text editing, accessibility roles, transitions,
reduced motion). Nothing from the styled `gpui-component` layer is used. The
workspace names the dependency `gpui`, because GPUI's macros expand to
`gpui::…` paths.

The app runs on `FakeBackend` until the rbw backend exists, and unlocks the
first demo account at start (`main.rs`).

## Build and run

```sh
cargo run -p rbw-desktop
RBW_DESKTOP_THEME=gnome-dark cargo run -p rbw-desktop   # macos, gnome, or either with -light / -dark
cargo run -p rbw-desktop-ui --example gallery
cargo fmt && cargo clippy --all-targets                 # clean before every commit
tools/check-macos                                       # type-check and lint the macOS build from Linux
```

On Ubuntu the build needs `libxkbcommon-dev libxkbcommon-x11-dev
libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev
libvulkan-dev libssl-dev libzstd-dev`; screenshots also need `xvfb xdotool
xcompmgr hsetroot x11-utils mesa-vulkan-drivers imagemagick ffmpeg`.

## Screenshots

Look at a screenshot after every visual change, next to the mockup it
implements.

```sh
tools/shot vault out.png                          # states: vault search logins select copied sheet gallery
tools/shot --theme gnome-dark sheet out.png
tools/shot --size 680x470 vault out.png           # the mockups' window size
tools/shot --then 'type "Netflix"' --then 'key ctrl+Return' sheet out.png
tools/shot --record 0.9 sheet out.png             # also out-frames/NNN.png at 30 fps, to check motion
```

It runs the app on a private Xvfb with Mesa's software Vulkan at 2x, over a
wallpaper with `xcompmgr`, reaches the state with real keys and clicks, and
crops around the window. `--then` runs `xdotool` with `$window` and `$scale`
set; pointer positions are in device pixels from the window's top-left, which
on Linux includes a 12-point invisible frame border. Without a window manager
GPUI does not see the pointer leave, so the script rests it on an empty spot
before capturing.

## Patterns

**A screen** is a view entity holding its state, `impl Render`, and methods
for its actions; long renders split into `render_*` methods in submodules
(`vault/sidebar.rs`, `vault/list.rs`). A part that redraws on its own, like
the item detail with its live code, is an entity of its own. Declare the
screen's actions with `actions!`, bind keys under the screen's key context in
its `init`, and handle them with `.on_action(cx.listener(Self::method))`.

**Calling the backend.** Every `Backend` call blocks. Use
`backend::request(cx, |backend| backend.items(&id), |this, result, cx| …)`:
the call runs on the background executor and the result comes back to the
view. Keep the returned `Task` in a field when a newer request should cancel
an older one (`load_task`), or `.detach()` it. Use `backend::request_in` when
the answer needs the window, such as setting a text field. Show data as
`Load<T>`. Keep the previous content on screen while new data loads rather
than blanking it (`ItemDetail::loaded`). Show a failure as the sentence from
`backend::describe(&error)`: in place where data failed to load, in the
window's `ToastSlot` when an action failed. Match on `Error` variants, never
on their text.

**Sheets.** The window view keeps a `SheetSlot` and renders it as one of its
last children; `present(entity)` shows a sheet and `dismiss()` animates it out
and then drops it. The sheet view binds `escape` to its own Cancel action and
calls back into the window view to dismiss (`ItemEditor::close`).

**Popovers** inside a window: `deferred(anchored().child(surface(id,
Material::Popover)…))` placed inside the element it hangs from, as the
editor's folder menu does.

**A keyboard shortcut and a menu item.** Add the action to the screen's
`actions!`, bind it in `init` with `KeyBinding::new(secondary!("e"), …)`
(`secondary!` is Command on macOS and Control elsewhere), and add
`MenuItem::action` to `menus()` in `main.rs`.

**Theme and motion.** Colours, sizes and durations come from
`cx.theme()` and `rbw_desktop_ui::motion`; app code holds none of its own. A
new colour goes into `Colors` for all four palettes. Motion uses the
`motion` helpers (`fade`, `movement`, `presence`, `content_enter`, the
`SELECTION` spring), which follow the system's reduced-motion setting.

## GPUI gotchas

- Updating or reading an entity from a listener that runs inside that
  entity's update panics ("already being updated"). To reach a parent view
  from a child's listener, dispatch an action (`EditItem`).
- `svg()` paints only with its own text colour. `ui::Icon` falls back to the
  inherited one.
- `Hsla::opacity(x)` multiplies the alpha; don't pass `color.a * x`.
- GPUI clips to rectangles. A pane that paints at a rounded window corner
  rounds itself with `chrome::corner_radius`.
- There is no backdrop blur, so sheet and popover surfaces are opaque.
- GPUI's Linux text system draws variable fonts (Adwaita Sans, Cantarell) at
  their default weight only. The UI bundles static Inter, which Adwaita Sans
  derives from.
- `KeyBinding::new` doesn't understand `secondary-`; use `secondary!`.
- `uniform_list` rows must all have the height of the first one.
- Element ids must be stable: transitions and other keyed state live under
  them. Two rows with the same id share a transition.
- On X11, GPUI draws its own frame only when it finds a compositing window
  manager with GTK frame extents; its compositor check looks up the wrong
  selection name, so `tools/shot` advertises one on the root window.
- `gpui-pre-apple`'s build script checks the host OS, so checking for macOS
  from Linux needs the empty shader file `tools/check-macos` creates.

## macOS

The macOS-only code is small and type-checked from Linux by
`tools/check-macos`, but none of it has run:

- `ui/src/glass.rs`: the `NSGlassEffectView` under the sidebar (macOS 26+),
  placed below GPUI's view and moved with the sidebar's bounds.
- `ui/src/platform.rs`: the accent colour from `NSColor.controlAccentColor`.
- `ui/src/chrome.rs`: transparent title bar, traffic lights at (21, 21) inside
  the sidebar, the `Blurred` window background.
- Fonts: `.SystemUIFont` and Menlo.

Everything else, the macOS look included, is the code Linux runs and is
checked with `RBW_DESKTOP_THEME=macos-light` / `macos-dark` screenshots.
