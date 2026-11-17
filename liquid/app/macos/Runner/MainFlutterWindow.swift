import Carbon.HIToolbox
import Cocoa
import FlutterMacOS
import PDFKit
import ServiceManagement

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)
    // Every tab fits from this size on, so the window shrinks no further; MainMenu.xib opens it
    // at this size.
    self.contentMinSize = NSSize(width: 1050, height: 640)

    RegisterGeneratedPlugins(registry: flutterViewController)
    FlutterMethodChannel(
      name: "liquid/liquid_key", binaryMessenger: flutterViewController.engine.binaryMessenger
    ).setMethodCallHandler(liquidKey)
    FlutterMethodChannel(
      name: "liquid/clipboard", binaryMessenger: flutterViewController.engine.binaryMessenger
    ).setMethodCallHandler(readClipboard)
    FlutterMethodChannel(
      name: "liquid/pdf", binaryMessenger: flutterViewController.engine.binaryMessenger
    ).setMethodCallHandler(pdfPages)
    logKeysIfAsked()

    super.awakeFromNib()
  }
}

/// With LIQUID_KEY_LOG=1, writes the key events macOS hands the window with the Liquid key
/// (F18), or with ⌘ or ⌃ but not ⌥, to stderr, beside Flutter's own log of them, to find why
/// some Liquid key shortcuts do nothing (issue #63). Plain typing stays out of the log: ⌥ types
/// characters such as @ on many layouts, and only Caps Lock of the modifier keys is logged. A
/// typed key's repeat or key-up is logged only when its key-down was, as `KeyLog` in
/// `key_log.dart` decides; this mirrors that rule, which has no test seam here.
private func logKeysIfAsked() {
  guard ProcessInfo.processInfo.environment["LIQUID_KEY_LOG"] == "1" else { return }
  let f18 = UInt16(kVK_F18)
  var liquidKeyDown = false
  // The key codes whose key-down was logged and that are not released yet.
  var loggedKeys = Set<UInt16>()
  // A Liquid key let go in another app never reaches this window.
  NotificationCenter.default.addObserver(
    forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
  ) { _ in liquidKeyDown = false }
  NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { event in
    if event.type != .flagsChanged && event.keyCode == f18 {
      liquidKeyDown = event.type == .keyDown
    }
    let code = event.keyCode
    let chord =
      !event.modifierFlags.intersection([.command, .control]).isEmpty
      && !event.modifierFlags.contains(.option)
    let logged: Bool
    switch event.type {
    case .flagsChanged:
      logged = liquidKeyDown || code == f18 || code == UInt16(kVK_CapsLock)
    case .keyDown where !event.isARepeat:
      logged = liquidKeyDown || code == f18 || chord
      if logged { loggedKeys.insert(code) } else { loggedKeys.remove(code) }
    case .keyDown:
      logged = code == f18 || loggedKeys.contains(code)
    default:
      logged = code == f18 || loggedKeys.remove(code) != nil
    }
    guard logged else { return event }
    let characters = event.type == .flagsChanged ? "-" : (event.characters ?? "-")
    let plain = event.type == .flagsChanged ? "-" : (event.charactersIgnoringModifiers ?? "-")
    FileHandle.standardError.write(
      Data(
        ("native key: \(event.type.rawValue) code \(event.keyCode) chars \"\(characters)\" "
          + "plain \"\(plain)\" flags 0x\(String(event.modifierFlags.rawValue, radix: 16)) "
          + "liquid \(liquidKeyDown) repeat \(event.type == .keyDown && event.isARepeat)\n").utf8))
    return event
  }
}

/// What the clipboard holds for a paste into a Text Block (ADR-0305), which Flutter's own
/// clipboard, plain text only, cannot tell: copied files' paths, an image as PNG, the editor
/// data VS Code and its forks put beside copied code, HTML, and the plain text.
private func readClipboard(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
  guard call.method == "read" else { return result(FlutterMethodNotImplemented) }
  let board = NSPasteboard.general
  var read: [String: Any] = [:]
  let urls =
    board.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
    as? [URL] ?? []
  if !urls.isEmpty { read["files"] = urls.map { $0.path } }
  if let png = board.data(forType: .png) {
    read["image"] = FlutterStandardTypedData(bytes: png)
  } else if let tiff = board.data(forType: .tiff),
    let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
  {
    read["image"] = FlutterStandardTypedData(bytes: png)
  }
  read["vscode"] = board.string(forType: NSPasteboard.PasteboardType("vscode-editor-data"))
  read["html"] = board.string(forType: .html)
  read["text"] = board.string(forType: .string)
  result(read)
}

/// The PDFs open in the renderer, by the id Dart holds, each with the queue that draws its
/// pages one at a time, as PDFKit promises no more; only the main thread touches the map.
private var pdfDocuments: [Int: (document: PDFDocument, queue: DispatchQueue)] = [:]
private var nextPDFId = 1

/// Liquid's PDF renderer (ADR-0307): PDFKit reads each file and draws its pages, so Apple's
/// maintained code parses them. `open` parses off the main thread and answers each page's size
/// in points; `render` draws one page off the main thread, as RGBA pixels; `close` drops a
/// document, and an unknown id is ignored.
private func pdfPages(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
  let arguments = call.arguments as? [String: Any]
  switch call.method {
  case "open":
    let bytes = (arguments?["bytes"] as? FlutterStandardTypedData)?.data ?? Data()
    // Parsing a large file takes a while, so it runs off the main thread, as `render` does.
    DispatchQueue.global(qos: .userInitiated).async {
      guard let document = PDFDocument(data: bytes), !document.isLocked, document.pageCount > 0
      else {
        DispatchQueue.main.async {
          result(FlutterError(code: "unreadable", message: "the PDF cannot be read", details: nil))
        }
        return
      }
      let sizes = (0..<document.pageCount).map { index -> [Double] in
        let size = document.page(at: index)?.displaySize ?? .zero
        return [Double(size.width), Double(size.height)]
      }
      DispatchQueue.main.async {
        let id = nextPDFId
        nextPDFId += 1
        pdfDocuments[id] = (document, DispatchQueue(label: "liquid.pdf.\(id)", qos: .userInitiated))
        result(["id": id, "sizes": sizes])
      }
    }
  case "render":
    guard let id = arguments?["id"] as? Int, let (document, queue) = pdfDocuments[id],
      let index = arguments?["page"] as? Int, index >= 0, index < document.pageCount
    else {
      return result(FlutterError(code: "notFound", message: "no such PDF page", details: nil))
    }
    // The width bounds the bitmap's size; the answer carries the width drawn.
    let width = min(max((arguments?["width"] as? Int) ?? 1, 1), 4096)
    queue.async {
      let drawn = document.page(at: index).flatMap { pdfPixels(of: $0, width: width) }
      DispatchQueue.main.async {
        guard let drawn else {
          return result(
            FlutterError(code: "render", message: "the page cannot be drawn", details: nil))
        }
        result([
          "width": drawn.width, "height": drawn.height,
          "pixels": FlutterStandardTypedData(bytes: drawn.pixels),
        ])
      }
    }
  case "close":
    if let id = arguments?["id"] as? Int { pdfDocuments[id] = nil }
    result(nil)
  default:
    result(FlutterMethodNotImplemented)
  }
}

private extension PDFPage {
  /// The page's size in points as displayed: its crop box, turned a quarter for a rotation of
  /// 90 or 270 degrees.
  var displaySize: CGSize {
    let box = bounds(for: .cropBox)
    return rotation % 180 == 0 ? box.size : CGSize(width: box.height, height: box.width)
  }
}

/// `page` drawn `width` pixels wide on white, as straight RGBA, 8 bits, row-major, without
/// padding; a tall page is drawn narrower, so it is never more than 4,096 pixels high. PDFKit
/// draws the displayed crop box from the context's origin, so scaling the context is all the
/// mapping needed.
private func pdfPixels(of page: PDFPage, width asked: Int) -> (width: Int, height: Int, pixels: Data)? {
  let size = page.displaySize
  guard size.width > 0, size.height > 0 else { return nil }
  let tallest = Double(4096) * size.width / size.height
  let width = max(1, min(asked, Int(tallest)))
  let height = min(4096, max(1, Int((Double(width) * size.height / size.width).rounded())))
  guard
    let context = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  else { return nil }
  // The page is opaque after the white fill, so premultiplied and straight RGBA agree.
  context.setFillColor(CGColor(gray: 1, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: width, height: height))
  context.scaleBy(x: CGFloat(width) / size.width, y: CGFloat(height) / size.height)
  page.draw(with: .cropBox, to: context)
  // The context owns the pixels, so it must live until they are copied.
  return withExtendedLifetime(context) { () -> (width: Int, height: Int, pixels: Data)? in
    guard let pixels = context.data else { return nil }
    return (width, height, Data(bytes: pixels, count: width * height * 4))
  }
}

/// Turns the Liquid key on or off: the login item inside the app bundle remaps
/// Caps Lock and listens for it, also after a restart or for keyboards attached later;
/// `--remove` gives Caps Lock back. The work runs off the main thread, so a slow helper never
/// stalls the window.
private func liquidKey(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
  guard call.method == "enable" || call.method == "disable" else {
    return result(FlutterMethodNotImplemented)
  }
  guard #available(macOS 13, *) else {
    return result(
      FlutterError(code: "unsupported", message: "the Liquid key needs macOS 13", details: nil))
  }
  let enable = call.method == "enable"
  DispatchQueue.global(qos: .userInitiated).async {
    let error = enable ? enableLiquidKey() : disableLiquidKey()
    DispatchQueue.main.async { result(error) }
  }
}

private let helperIdentifier = Bundle.main.bundleIdentifier! + ".key"

@available(macOS 13, *)
private var loginItem: SMAppService {
  SMAppService.loginItem(identifier: helperIdentifier)
}

/// Whether the helper runs; it quits, for example, when another app took its key.
private var helperRunning: Bool {
  !NSRunningApplication.runningApplications(withBundleIdentifier: helperIdentifier).isEmpty
}

@available(macOS 13, *)
private func enableLiquidKey() -> FlutterError? {
  let item = loginItem
  let approval = FlutterError(
    code: "approval", message: "allow Liquid Key in System Settings → General → Login Items",
    details: nil)
  do {
    if item.status == .requiresApproval { return approval }
    // A registered helper that quit starts again.
    if item.status == .enabled && !helperRunning { try item.unregister() }
    if item.status != .enabled { try item.register() }
    return item.status == .requiresApproval ? approval : nil
  } catch {
    return FlutterError(code: "enable", message: error.localizedDescription, details: nil)
  }
}

/// Caps Lock comes back even when the login item cannot be removed.
@available(macOS 13, *)
private func disableLiquidKey() -> FlutterError? {
  let item = loginItem
  var failure: Error?
  if item.status == .enabled || item.status == .requiresApproval {
    do { try item.unregister() } catch { failure = error }
  }
  // A helper launchd keeps running would map Caps Lock again; asked to stop, it gives it back.
  if failure != nil {
    for helper in NSRunningApplication.runningApplications(withBundleIdentifier: helperIdentifier) {
      kill(helper.processIdentifier, SIGTERM)
    }
  }
  do { try runHelper("--remove") } catch { failure = failure ?? error }
  return failure.map {
    FlutterError(code: "disable", message: $0.localizedDescription, details: nil)
  }
}

struct HelperFailed: LocalizedError {
  let errorDescription: String?
}

/// What a refused helper run reports: what the helper said, or else its exit status.
func helperRefusal(status: Int32, said: Data) -> HelperFailed {
  let message = String(decoding: said, as: UTF8.self).trimmingCharacters(
    in: .whitespacesAndNewlines)
  return HelperFailed(
    errorDescription: message.isEmpty
      ? "the Liquid key helper failed with status \(status)" : message)
}

/// Runs the helper once; one that does not finish within ten seconds is stopped. A refusal
/// quotes what the helper said, or else its exit status.
private func runHelper(_ argument: String) throws {
  let helper = Bundle.main.bundleURL.appendingPathComponent(
    "Contents/Library/LoginItems/LiquidKey.app/Contents/MacOS/LiquidKey")
  let finished = DispatchSemaphore(value: 0)
  let run = Process()
  let said = Pipe()
  run.executableURL = helper
  run.arguments = [argument]
  run.standardError = said
  run.terminationHandler = { _ in finished.signal() }
  try run.run()
  if finished.wait(timeout: .now() + 10) == .timedOut {
    run.terminate()
    throw HelperFailed(errorDescription: "the Liquid key helper did not finish")
  }
  if run.terminationStatus != 0 {
    throw helperRefusal(
      status: run.terminationStatus, said: said.fileHandleForReading.readDataToEndOfFile())
  }
}
