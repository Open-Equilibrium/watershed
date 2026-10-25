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
/// Caps Lock and listens for it; `--apply` and `--remove` set or give back the remapping, which a
/// restart or a newly attached keyboard can lose.
private func liquidKey(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
  guard call.method == "enable" || call.method == "disable" else {
    return result(FlutterMethodNotImplemented)
  }
  guard #available(macOS 13, *) else {
    return result(
      FlutterError(code: "unsupported", message: "the Liquid key needs macOS 13", details: nil))
  }
  let item = SMAppService.loginItem(identifier: Bundle.main.bundleIdentifier! + ".key")
  let approval = FlutterError(
    code: "approval", message: "allow Liquid Key in System Settings → General → Login Items",
    details: nil)
  do {
    if call.method == "enable" {
      if item.status == .requiresApproval { return result(approval) }
      if item.status != .enabled { try item.register() }
      if item.status == .requiresApproval { return result(approval) }
      try runHelper("--apply")
    } else {
      // Caps Lock comes back even when the login item cannot be removed.
      var failure: Error?
      if item.status == .enabled || item.status == .requiresApproval {
        do { try item.unregister() } catch { failure = error }
      }
      try runHelper("--remove")
      if let failure { throw failure }
    }
    result(nil)
  } catch {
    result(FlutterError(code: call.method, message: error.localizedDescription, details: nil))
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
