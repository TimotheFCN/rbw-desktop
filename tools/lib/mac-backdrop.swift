// A busy, colourful desktop for screenshots of translucent windows on a Mac,
// where CI's own desktop is black: bright and dark bands and discs over the
// whole screen, under every window. With --front it is instead a small
// window that takes the keyboard, so that the app's windows show inactive.
// tools/mac-check compiles and runs it.
import AppKit

let front = CommandLine.arguments.contains("--front")
let app = NSApplication.shared
app.setActivationPolicy(front ? .regular : .accessory)
let screen = NSScreen.main!.frame

func layer(_ path: CGPath, _ rgb: UInt32) -> CAShapeLayer {
    let shape = CAShapeLayer()
    shape.path = path
    shape.fillColor = CGColor(
        red: CGFloat((rgb >> 16) & 0xff) / 255,
        green: CGFloat((rgb >> 8) & 0xff) / 255,
        blue: CGFloat(rgb & 0xff) / 255,
        alpha: 1)
    return shape
}

let window: NSWindow
if front {
    window = NSWindow(
        contentRect: NSRect(x: screen.maxX - 220, y: 90, width: 200, height: 80),
        styleMask: [.titled], backing: .buffered, defer: false)
    window.title = "Another app"
} else {
    window = NSWindow(contentRect: screen, styleMask: .borderless, backing: .buffered, defer: false)
    window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
    window.collectionBehavior = [.canJoinAllSpaces, .stationary]
    let view = NSView(frame: screen)
    view.wantsLayer = true
    let root = CALayer()
    root.backgroundColor = CGColor(red: 0.05, green: 0.06, blue: 0.16, alpha: 1)
    // Diagonal bands, each a different colour, some very dark.
    let colors: [UInt32] = [0xff6a00, 0x1b1f3b, 0xe8318a, 0xffd23f, 0x0b0b0f, 0x16a6b6, 0x7b2ff7, 0x2b9348]
    let band = screen.width / 5
    for (index, color) in colors.enumerated() {
        let x = CGFloat(index) * band * 0.8 - screen.height * 0.5
        let path = CGMutablePath()
        path.addLines(between: [
            CGPoint(x: x, y: 0), CGPoint(x: x + band * 0.8, y: 0),
            CGPoint(x: x + band * 0.8 + screen.height, y: screen.height),
            CGPoint(x: x + screen.height, y: screen.height),
        ])
        root.addSublayer(layer(path, color))
    }
    // Discs of high contrast, to show what blurring does to edges.
    for (center, radius, color) in [
        (CGPoint(x: 140, y: 520), 110.0, 0xffffff as UInt32),
        (CGPoint(x: 260, y: 220), 90.0, 0x000000),
        (CGPoint(x: 820, y: 600), 140.0, 0x00e5ff),
        (CGPoint(x: 600, y: 160), 120.0, 0xff1744),
    ] {
        let path = CGPath(
            ellipseIn: CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2),
            transform: nil)
        root.addSublayer(layer(path, color))
    }
    view.layer = root
    window.contentView = view
}
window.orderFrontRegardless()
if front {
    window.makeKey()
    app.activate(ignoringOtherApps: true)
}
app.run()
