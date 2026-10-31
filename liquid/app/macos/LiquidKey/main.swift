import AppKit
import Carbon.HIToolbox
import IOKit
import IOKit.hidsystem

// The Liquid key helper: a login item inside Liquid.app. It turns Caps Lock into the
// Liquid key on every keyboard, opens or focuses Liquid when the key is tapped in another app
// and turns capitals on or off with Shift + Caps Lock. It needs no permission. `--remove` gives
// the remapping back and exits. The remapping holds for the whole computer, so the helper
// gives it back when it quits or the user switches away.

if CommandLine.arguments.dropFirst().first == "--remove" {
    Mapping.apply(on: false)
    exit(0)
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
var lastPress: TimeInterval = 0
var shiftedDown = false
/// Whether this user's session is in front; the remapping holds for every user's keyboards.
var sessionActive = true

/// Sets the remapping while this session is in front.
func remap() {
    if sessionActive { Mapping.apply(on: true) }
}

/// Gives the remapping back if this session set it; another user's helper may hold it now.
func unmap() {
    if sessionActive { Mapping.apply(on: false) }
}

/// Without its hotkeys the key would be dead, so Caps Lock comes back and the helper ends.
/// Registering exclusively fails while any other app holds the same keys, which a plain
/// registration would accept and then never hear.
func register(_ modifiers: Int, _ id: EventHotKeyID, _ key: inout EventHotKeyRef?) {
    let status = RegisterEventHotKey(
        UInt32(kVK_F18), UInt32(modifiers), id, GetApplicationEventTarget(),
        OptionBits(kEventHotKeyExclusive), &key)
    if status != noErr {
        FileHandle.standardError.write(Data("Liquid Key: hotkey taken (\(status))\n".utf8))
        unmap()
        exit(1)
    }
}

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
        register(0, plainID, &plain)
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
        case (plainID.id, true):
            pressedAt = pressedAt ?? now
            lastPress = now
        case (plainID.id, false):
            // The time since any key went down tells whether another one did after this one.
            let lastKeyDown =
                now - CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .keyDown)
            if let at = pressedAt,
                isTap(pressed: at, released: now, lastPress: lastPress, lastKeyDown: lastKeyDown)
            {
                openLiquid()
            }
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
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { remap() }
        }
    }, nil, &attached)
drain(attached)

// The system stops a login item that the user turns off with a termination signal. Handled
// before the first remapping, a signal during startup gives it back once the helper runs.
signal(SIGTERM, SIG_IGN)
let terminate = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
terminate.setEventHandler {
    unmap()
    exit(0)
}
terminate.resume()

register(shiftKey, shiftedID, &shifted)
follow(NSWorkspace.shared.frontmostApplication)
remap()
let workspace = NSWorkspace.shared.notificationCenter
workspace.addObserver(
    forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
) { note in
    follow(note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)
}
workspace.addObserver(
    forName: NSWorkspace.sessionDidResignActiveNotification, object: nil, queue: .main
) { _ in
    sessionActive = false
    Mapping.apply(on: false)
}
workspace.addObserver(
    forName: NSWorkspace.sessionDidBecomeActiveNotification, object: nil, queue: .main
) { _ in
    sessionActive = true
    remap()
}
NotificationCenter.default.addObserver(
    forName: NSApplication.willTerminateNotification, object: nil, queue: .main
) { _ in unmap() }

NSApplication.shared.setActivationPolicy(.prohibited)
NSApplication.shared.run()
