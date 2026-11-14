import AppKit

// Synthetic external provider: no AXSelectedText, so Nami must use copy fallback.
final class FixtureWindow: NSWindow {
  var mode = "Text"
  override func performKeyEquivalent(with event: NSEvent) -> Bool {
    if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers == "c" {
      title = "Nami Synthetic Selection — Copied \(mode)"
      guard mode != "Empty" else { return true }
      let item = NSPasteboardItem()
      item.setString("Nami synthetic copy fallback test.", forType: .string)
      if mode == "Image" { item.setData(Data([0, 1, 2]), forType: .png) }
      if mode == "File" { item.setString("file:///tmp/nami-synthetic-file.txt", forType: .fileURL) }
      NSPasteboard.general.clearContents()
      NSPasteboard.general.writeObjects([item])
      return true
    }
    if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers == "t" {
      title = "Nami Synthetic Selection — T reached source"
      return true
    }
    return super.performKeyEquivalent(with: event)
  }
}

final class Fixture: NSObject {
  let window = FixtureWindow(contentRect: NSRect(x: 120, y: 200, width: 540, height: 220), styleMask: [.titled, .closable], backing: .buffered, defer: false)
  @objc func choose(_ sender: NSButton) {
    window.mode = sender.title
    window.title = "Nami Synthetic Selection — \(sender.title)"
    window.makeFirstResponder(window.contentView)
  }
  func start() {
    window.title = "Nami Synthetic Selection — Text"
    let label = NSTextField(labelWithString: "Synthetic selection: Nami synthetic copy fallback test.")
    label.frame = NSRect(x: 20, y: 145, width: 500, height: 40)
    window.contentView?.addSubview(label)
    for (index, title) in ["Text", "Image", "File", "Empty"].enumerated() {
      let button = NSButton(title: title, target: self, action: #selector(choose(_:)))
      button.frame = NSRect(x: 20 + index * 125, y: 80, width: 110, height: 30)
      window.contentView?.addSubview(button)
    }
    window.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }
}
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let fixture = Fixture()
fixture.start()
app.run()
