import AppKit

// Match FinderX's monochrome, regular 16pt history symbols at 3x resolution.
let output = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .deletingLastPathComponent().appendingPathComponent("src/assets/history")
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
for name in ["list.bullet", "star", "star.fill", "pin", "pin.fill", "arrow.clockwise", "doc.on.doc", "trash", "magnifyingglass", "xmark.circle.fill"] {
    guard let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
        .withSymbolConfiguration(.init(pointSize: 16, weight: .regular))?
        .withSymbolConfiguration(.preferringMonochrome()),
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 48, pixelsHigh: 48,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
        let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
        fatalError("Cannot render SF Symbol: \(name)")
    }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    let scale = min(48 / image.size.width, 48 / image.size.height)
    let size = NSSize(width: image.size.width * scale, height: image.size.height * scale)
    image.draw(in: NSRect(x: (48 - size.width) / 2, y: (48 - size.height) / 2,
        width: size.width, height: size.height))
    NSGraphicsContext.restoreGraphicsState()
    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        fatalError("Cannot encode SF Symbol: \(name)")
    }
    try png.write(to: output.appendingPathComponent("\(name).png"))
}
