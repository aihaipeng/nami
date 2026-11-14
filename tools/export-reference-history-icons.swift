import AppKit

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let output = root.appendingPathComponent("src/assets/history")
let reference = output.appendingPathComponent("reference")

for (name, source, filled) in [
    ("reference-clock", "clock", false),
    ("reference-keyboard", "keyboard", false),
    ("reference-list", "list", false),
    ("reference-refresh", "refresh-cw", false),
    ("reference-copy", "copy", false),
    ("reference-trash", "trash", false),
    ("reference-search", "search", false),
    ("reference-clear", "clear", false),
    ("reference-star", "star", false),
    ("reference-star-filled", "star", true),
    ("reference-pin", "pin", false),
    ("reference-pin-filled", "pin", true)
] {
    var svg = try String(contentsOf: reference.appendingPathComponent("\(source).svg"), encoding: .utf8)
        .replacingOccurrences(of: "currentColor", with: "black")
    if filled {
        let path = source == "star" ? "<path d=\"M11.525" : "<path d=\"M9 10.76"
        svg = svg.replacingOccurrences(of: path, with: path.replacingOccurrences(of: "<path", with: "<path fill=\"black\""))
    }
    guard let image = NSImage(data: Data(svg.utf8)),
          let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 48, pixelsHigh: 48,
              bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
              colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
          let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
        fatalError("Cannot render reference SVG: \(source)")
    }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    image.draw(in: NSRect(x: 0, y: 0, width: 48, height: 48))
    NSGraphicsContext.restoreGraphicsState()
    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        fatalError("Cannot encode reference image: \(name)")
    }
    try png.write(to: output.appendingPathComponent("\(name).png"))
}
