import AppKit
import Carbon.HIToolbox
import IOKit
import IOKit.hidsystem

// The Liquid key helper (ADR-0205): a login item inside Liquid.app. It turns Caps Lock into the
// Liquid key on every keyboard, opens or focuses Liquid when the key is tapped in another app
// and turns capitals on or off with Shift + Caps Lock. It needs no permission. `--apply` and
// `--remove` set or give back the remapping and exit. The remapping holds for the whole
// computer, so the helper gives it back when it quits or the user switches away.

switch CommandLine.arguments.dropFirst().first {
case "--apply":
    Mapping.apply(on: true)
    exit(0)
case "--remove":
    Mapping.apply(on: false)
    exit(0)
default:
    break
}

/// Liquid.app, four levels above Contents/Library/LoginItems/LiquidKey.app.
let liquid = Bundle.main.bundleURL
    .deletingLastPathComponent().deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()
let liquidID = Bundle(url: liquid)?.bundleIdentifier

let plainID = EventHotKeyID(signature: 0x4C51_4B59, id: 1)
let shiftedID = EventHotKeyID(signature: 0x4C51_4B59, id: 2)
var plain: EventHotKeyRef?
var shifted: EventHotKeyRef?
var pressedAt: TimeInterval?
var shiftedDown = false

func openLiquid() {
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.activates = true
    NSWorkspace.shared.openApplication(at: liquid, configuration: configuration)
}

func toggleCapsLock() {
    let system = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching(kIOHIDSystemClass))
    defer { IOObjectRelease(system) }
    var connect: io_connect_t = 0
    guard IOServiceOpen(system, mach_task_self_, UInt32(kIOHIDParamConnectType), &connect)
        == KERN_SUCCESS
    else { return }
    defer { IOServiceClose(connect) }
    var on = false
    IOHIDGetModifierLockState(connect, Int32(kIOHIDCapsLockState), &on)
    IOHIDSetModifierLockState(connect, Int32(kIOHIDCapsLockState), !on)
}

/// Liquid takes the key itself while it is in front, for holding and ⇪ shortcuts.
func follow(_ app: NSRunningApplication?) {
    let inLiquid = liquidID != nil && app?.bundleIdentifier == liquidID
    if inLiquid, let key = plain {
        UnregisterEventHotKey(key)
        plain = nil
        pressedAt = nil
    } else if !inLiquid, plain == nil {
        RegisterEventHotKey(
            UInt32(kVK_F18), 0, plainID, GetApplicationEventTarget(), 0, &plain)
    }
}

var kinds = [
    EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
    EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased)),
]
InstallEventHandler(
    GetApplicationEventTarget(),
    { _, event, _ in
        var id = EventHotKeyID()
        GetEventParameter(
            event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
            nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
        let pressed = GetEventKind(event) == UInt32(kEventHotKeyPressed)
        let now = ProcessInfo.processInfo.systemUptime
        // A held key may repeat its press; only the first one counts.
        switch (id.id, pressed) {
        case (plainID.id, true): pressedAt = pressedAt ?? now
        case (plainID.id, false):
            if let at = pressedAt, isTap(pressed: at, released: now) { openLiquid() }
            pressedAt = nil
        case (shiftedID.id, true):
            if !shiftedDown { toggleCapsLock() }
            shiftedDown = true
        case (shiftedID.id, false): shiftedDown = false
        default: break
        }
        return noErr
    }, kinds.count, &kinds, nil, nil)

/// Keyboards attached later get the Liquid key too, once their HID service exists.
let arrivals = IONotificationPortCreate(kIOMainPortDefault)
CFRunLoopAddSource(
    CFRunLoopGetMain(), IONotificationPortGetRunLoopSource(arrivals).takeUnretainedValue(),
    .defaultMode)
var attached: io_iterator_t = 0
func drain(_ iterator: io_iterator_t) {
    while case let device = IOIteratorNext(iterator), device != 0 { IOObjectRelease(device) }
}
IOServiceAddMatchingNotification(
    arrivals, kIOFirstMatchNotification, IOServiceMatching("IOHIDInterface"),
    { _, iterator in
        drain(iterator)
        // A keyboard's HID service appears some time after the device, a Bluetooth one later.
        for delay in [1.0, 5.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { Mapping.apply(on: true) }
        }
    }, nil, &attached)
drain(attached)

Mapping.apply(on: true)
RegisterEventHotKey(
    UInt32(kVK_F18), UInt32(shiftKey), shiftedID, GetApplicationEventTarget(), 0, &shifted)
follow(NSWorkspace.shared.frontmostApplication)
let workspace = NSWorkspace.shared.notificationCenter
workspace.addObserver(
    forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
) { note in
    follow(note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)
}
workspace.addObserver(
    forName: NSWorkspace.sessionDidResignActiveNotification, object: nil, queue: .main
) { _ in Mapping.apply(on: false) }
workspace.addObserver(
    forName: NSWorkspace.sessionDidBecomeActiveNotification, object: nil, queue: .main
) { _ in Mapping.apply(on: true) }
NotificationCenter.default.addObserver(
    forName: NSApplication.willTerminateNotification, object: nil, queue: .main
) { _ in Mapping.apply(on: false) }

NSApplication.shared.setActivationPolicy(.prohibited)
NSApplication.shared.run()
