import Cocoa
import FlutterMacOS
import ServiceManagement

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    FlutterMethodChannel(
      name: "liquid/liquid_key", binaryMessenger: flutterViewController.engine.binaryMessenger
    ).setMethodCallHandler(liquidKey)

    super.awakeFromNib()
  }
}

/// Turns the Liquid key on or off (ADR-0205): the login item inside the app bundle remaps
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

@available(macOS 13, *)
private var loginItem: SMAppService {
  SMAppService.loginItem(identifier: Bundle.main.bundleIdentifier! + ".key")
}

@available(macOS 13, *)
private func enableLiquidKey() -> FlutterError? {
  let item = loginItem
  let approval = FlutterError(
    code: "approval", message: "allow Liquid Key in System Settings → General → Login Items",
    details: nil)
  do {
    if item.status == .requiresApproval { return approval }
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
  let status: Int32
  var errorDescription: String? { "the Liquid key helper failed with status \(status)" }
}

private func runHelper(_ argument: String) throws {
  let helper = Bundle.main.bundleURL.appendingPathComponent(
    "Contents/Library/LoginItems/LiquidKey.app/Contents/MacOS/LiquidKey")
  let run = try Process.run(helper, arguments: [argument])
  run.waitUntilExit()
  if run.terminationStatus != 0 { throw HelperFailed(status: run.terminationStatus) }
}
