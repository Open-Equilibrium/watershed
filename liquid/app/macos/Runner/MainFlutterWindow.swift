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

private func logKeysIfAsked() {
  guard ProcessInfo.processInfo.environment["LIQUID_KEY_LOG"] == "1" else { return }
  let f18 = UInt16(kVK_F18)
  var liquidKeyDown = false
  var loggedKeys = Set<UInt16>()
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
  read["change"] = board.changeCount
  result(read)
}

private var pdfDocuments: [Int: (document: PDFDocument, queue: DispatchQueue)] = [:]
private var nextPDFId = 1

private func pdfPages(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
  let arguments = call.arguments as? [String: Any]
  switch call.method {
  case "open":
    let bytes = (arguments?["bytes"] as? FlutterStandardTypedData)?.data ?? Data()
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
  var displaySize: CGSize {
    let box = bounds(for: .cropBox)
    return rotation % 180 == 0 ? box.size : CGSize(width: box.height, height: box.width)
  }
}

private func pdfPixels(of page: PDFPage, width asked: Int) -> (width: Int, height: Int, pixels: Data)? {
  let size = page.displaySize
  guard size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0
  else { return nil }
  let ratio = Double(size.height) / Double(size.width)
  let width = Int(min(Double(asked), max(1, (4096 / ratio).rounded(.down))))
  let height = Int(min(4096, max(1, (Double(width) * ratio).rounded())))
  guard
    let context = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  else { return nil }
  context.setFillColor(CGColor(gray: 1, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: width, height: height))
  context.scaleBy(x: CGFloat(width) / size.width, y: CGFloat(height) / size.height)
  page.draw(with: .cropBox, to: context)
  return withExtendedLifetime(context) { () -> (width: Int, height: Int, pixels: Data)? in
    guard let pixels = context.data else { return nil }
    return (width, height, Data(bytes: pixels, count: width * height * 4))
  }
}

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
    if item.status == .enabled && !helperRunning { try item.unregister() }
    if item.status != .enabled { try item.register() }
    return item.status == .requiresApproval ? approval : nil
  } catch {
    return FlutterError(code: "enable", message: error.localizedDescription, details: nil)
  }
}

@available(macOS 13, *)
private func disableLiquidKey() -> FlutterError? {
  let item = loginItem
  var failure: Error?
  if item.status == .enabled || item.status == .requiresApproval {
    do { try item.unregister() } catch { failure = error }
  }
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

func helperRefusal(status: Int32, said: Data) -> HelperFailed {
  let message = String(decoding: said, as: UTF8.self).trimmingCharacters(
    in: .whitespacesAndNewlines)
  return HelperFailed(
    errorDescription: message.isEmpty
      ? "the Liquid key helper failed with status \(status)" : message)
}

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
