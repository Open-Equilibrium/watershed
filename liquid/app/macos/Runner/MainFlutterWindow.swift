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
/// Caps Lock and listens for it; `--remove` gives Caps Lock back.
private func liquidKey(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
  guard call.method == "enable" || call.method == "disable" else {
    return result(FlutterMethodNotImplemented)
  }
  guard #available(macOS 13, *) else {
    return result(
      FlutterError(code: "unsupported", message: "the Liquid key needs macOS 13", details: nil))
  }
  let item = SMAppService.loginItem(identifier: Bundle.main.bundleIdentifier! + ".key")
  do {
    if call.method == "enable" {
      if item.status != .enabled { try item.register() }
      if item.status == .requiresApproval {
        return result(
          FlutterError(
            code: "approval",
            message: "allow Liquid Key in System Settings → General → Login Items", details: nil))
      }
    } else {
      if item.status == .enabled || item.status == .requiresApproval { try item.unregister() }
      let helper = Bundle.main.bundleURL.appendingPathComponent(
        "Contents/Library/LoginItems/LiquidKey.app/Contents/MacOS/LiquidKey")
      let remove = try Process.run(helper, arguments: ["--remove"])
      remove.waitUntilExit()
    }
    result(nil)
  } catch {
    result(FlutterError(code: call.method, message: error.localizedDescription, details: nil))
  }
}
