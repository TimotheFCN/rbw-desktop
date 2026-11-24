# The private headless display that tools/shot and tools/e2e run the app
# on: Xvfb with Mesa's software Vulkan, a wallpaper, xcompmgr, and the root
# window properties GPUI looks for before it draws its own frame. Source it
# from bash with `theme` and `scale` set, then call x11_start.
#
# Pointer positions are in points from a window's top-left corner, where
# Linux draws an invisible 12-point frame border.

frame=12
x11_pids=()

# Starts the display in a new work folder ($work), removed on exit with
# everything started here.
x11_start() {
	work=$(mktemp -d "${TMPDIR:-/tmp}/rbw-desktop-x11.XXXXXX")
	trap x11_stop EXIT
	# A screen with room for a window and its shadow at any supported size.
	screen_w=$((1400 * scale)) screen_h=$((1000 * scale))
	Xvfb -displayfd 3 -screen 0 "${screen_w}x${screen_h}x24" -nolisten tcp 3>"$work/display" 2>/dev/null &
	x11_pids+=($!)
	for _ in $(seq 50); do [[ -s $work/display ]] && break; sleep 0.1; done
	export DISPLAY=:$(<"$work/display")

	local colors
	case $theme in
	macos-light) colors=(-define gradient:angle=135 "gradient:#bcd5ff-#f4d9ec") ;;
	macos-dark) colors=(-define gradient:angle=135 "gradient:#1b2b63-#4a2350") ;;
	gnome-*) colors=(-define gradient:angle=160 "gradient:#2c5aa0-#77459a") ;;
	*) echo "unknown theme: $theme" >&2; return 1 ;;
	esac
	magick -size "${screen_w}x${screen_h}" "${colors[@]}" "$work/wallpaper.png"
	hsetroot -cover "$work/wallpaper.png" >/dev/null
	xcompmgr -n &
	x11_pids+=($!)
	# Let it claim the compositor selection before the app looks for it.
	sleep 0.5
	# GPUI draws its own frame on X11 only where it finds a compositing
	# window manager that supports GTK frame extents, as GNOME's does. There
	# is no window manager here, so advertise one on the root window.
	# (GPUI's check of the compositor selection names it after the root
	# window instead of the screen, so xcompmgr alone is not found.)
	python3 -I - <<'PY'
import ctypes
x = ctypes.cdll.LoadLibrary("libX11.so.6")
x.XOpenDisplay.restype = ctypes.c_void_p
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XChangeProperty.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong,
                              ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
display = x.XOpenDisplay(None)
root = x.XDefaultRootWindow(display)
ATOM, WINDOW = 4, 33
def set_property(name, kind, value):
    data = (ctypes.c_ulong * 1)(value)
    x.XChangeProperty(display, root, x.XInternAtom(display, name, 0), kind, 32, 0, data, 1)
set_property(b"_NET_SUPPORTING_WM_CHECK", WINDOW, root)
set_property(b"_NET_SUPPORTED", ATOM, x.XInternAtom(display, b"_GTK_FRAME_EXTENTS", 0))
x.XCloseDisplay(display)
PY
	# Keep the pointer off where windows will appear.
	xdotool mousemove 0 0
}

x11_stop() {
	for pid in "${x11_pids[@]}"; do kill "$pid" 2>/dev/null || true; done
	wait 2>/dev/null || true
	rm -rf "$work"
}

# Runs the app in the background, with its output in $work/app.log.
x11_launch() {
	RBW_DESKTOP_THEME=$theme GPUI_X11_SCALE_FACTOR=$scale RUST_LOG=${RUST_LOG:-error} \
		"$@" >>"$work/app.log" 2>&1 &
	x11_pids+=($!)
	app_pid=$!
}

x11_fail() {
	echo "$1; the app's log:" >&2
	cat "$work/app.log" >&2
	exit 1
}

# Waits for the window whose title matches the pattern, makes it $window,
# centres it and gives it the keyboard.
x11_window() {
	window=$(timeout "${2:-30}" xdotool search --sync --onlyvisible --name "$1" | head -1) ||
		x11_fail "no window '$1' appeared"
	x11_geometry
	xdotool windowmove "$window" $(((screen_w - win_w) / 2)) $(((screen_h - win_h) / 2))
	# Without a window manager, keyboard focus follows the pointer unless set.
	xdotool windowfocus --sync "$window"
}

# The window's position and size in pixels: win_x, win_y, win_w, win_h.
x11_geometry() {
	local shell
	shell=$(xdotool getwindowgeometry --shell "$window" 2>/dev/null) || x11_fail "the window closed"
	eval "$shell"
	win_x=$X win_y=$Y win_w=$WIDTH win_h=$HEIGHT
}

# Clicks at a point, in points from the window's top-left corner.
x11_click() {
	xdotool mousemove --window "$window" $((($1 + frame) * scale)) $((($2 + frame) * scale)) click 1
}

# Rests the pointer on the empty bottom-right of the window: X11 without a
# window manager doesn't tell the app when the pointer leaves it, so a
# pointer outside the window would leave a hover highlight behind.
x11_rest_pointer() {
	xdotool mousemove --window "$window" $((win_w - 40 * scale)) $((win_h - 40 * scale))
}

# Screenshots the window with a margin of wallpaper around it.
x11_capture() {
	local margin=$((40 * scale))
	x11_geometry
	mkdir -p "$(dirname "$1")"
	import -window root -crop "$((win_w + 2 * margin))x$((win_h + 2 * margin))+$((win_x - margin))+$((win_y - margin))" +repage "$1"
}
