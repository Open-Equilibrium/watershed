import Cocoa
import FlutterMacOS
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
    logKeysIfAsked()

    super.awakeFromNib()
  }
}

/// With LIQUID_KEY_LOG=1, writes the key events macOS hands the window while ⌘, ⌃, ⌥ or the
/// Liquid key (F18) is held to stderr, beside Flutter's own log of them, to find why some
/// Liquid key shortcuts do nothing (PLAN.md). Plain typing never lands in the log.
private func logKeysIfAsked() {
  guard ProcessInfo.processInfo.environment["LIQUID_KEY_LOG"] == "1" else { return }
  let f18: UInt16 = 79
  var liquidKeyDown = false
  NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { event in
    if event.type != .flagsChanged && event.keyCode == f18 {
      liquidKeyDown = event.type == .keyDown
    }
    let held = event.modifierFlags.intersection([.command, .control, .option])
    guard liquidKeyDown || !held.isEmpty || event.keyCode == f18 || event.type == .flagsChanged
    else { return event }
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
  do { try runHelper("--remove") } catch { failure = failure ?? error }
  return failure.map {
    FlutterError(code: "disable", message: $0.localizedDescription, details: nil)
  }
}

private struct HelperFailed: LocalizedError {
  let reason: String
  var errorDescription: String? { "the Liquid key helper \(reason)" }
}

/// Runs the helper once; one that does not finish within ten seconds is stopped.
private func runHelper(_ argument: String) throws {
  let helper = Bundle.main.bundleURL.appendingPathComponent(
    "Contents/Library/LoginItems/LiquidKey.app/Contents/MacOS/LiquidKey")
  let finished = DispatchSemaphore(value: 0)
  let run = Process()
  run.executableURL = helper
  run.arguments = [argument]
  run.terminationHandler = { _ in finished.signal() }
  try run.run()
  if finished.wait(timeout: .now() + 10) == .timedOut {
    run.terminate()
    throw HelperFailed(reason: "did not finish")
  }
  if run.terminationStatus != 0 {
    throw HelperFailed(reason: "failed with status \(run.terminationStatus)")
  }
}
