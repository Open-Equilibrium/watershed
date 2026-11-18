import AppKit
import Carbon.HIToolbox
import IOKit
import IOKit.hidsystem

if CommandLine.arguments.dropFirst().first == "--remove" {
    if Mapping.apply(on: false) { exit(0) }
    FileHandle.standardError.write(
        Data("a keyboard may keep Caps Lock as the Liquid key\n".utf8))
    exit(1)
}

let liquid = Bundle.main.bundleURL
    .deletingLastPathComponent().deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()
let liquidID = Bundle(url: liquid)?.bundleIdentifier

let plainID = EventHotKeyID(signature: 0x4C51_4B59, id: 1)
let shiftedID = EventHotKeyID(signature: 0x4C51_4B59, id: 2)
var plain: EventHotKeyRef?
var shifted: EventHotKeyRef?
var press: Press?
var shiftedDown = false
var sessionActive = false
var mapped = false

func remap() {
    if sessionActive {
        Mapping.apply(on: true)
        mapped = true
    }
}

func remapLater() {
    for delay in [1.0, 5.0] {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { remap() }
    }
}

var recheck: Timer?

func startRecheck() {
    stopRecheck()
    recheck = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in
        Mapping.reapply(active: sessionActive)
    }
}

func stopRecheck() {
    recheck?.invalidate()
    recheck = nil
}

func unmap() {
    stopRecheck()
    if Mapping.givesBack(mapped: mapped, active: sessionActive) {
        Mapping.apply(on: false)
        mapped = false
    }
}

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

func follow(_ app: NSRunningApplication?) {
    let inLiquid = liquidID != nil && app?.bundleIdentifier == liquidID
    if inLiquid, let key = plain {
        UnregisterEventHotKey(key)
        plain = nil
        press = nil
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
        let lastKeyDown =
            now - CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .keyDown)
        switch (id.id, pressed) {
        case (plainID.id, true):
            if press == nil {
                press = Press(at: now)
            } else {
                press?.repeated(at: now, lastKeyDown: lastKeyDown)
            }
        case (plainID.id, false):
            if press?.isTap(released: now, lastKeyDown: lastKeyDown) == true { openLiquid() }
            press = nil
        case (shiftedID.id, true):
            if !shiftedDown { toggleCapsLock() }
            shiftedDown = true
        case (shiftedID.id, false): shiftedDown = false
        default: break
        }
        return noErr
    }, kinds.count, &kinds, nil, nil)

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
        remapLater()
    }, nil, &attached)
drain(attached)

let terminate = DispatchSource.makeSignalSource(
    signal: SIGTERM, queue: DispatchQueue(label: "terminate"))
terminate.setEventHandler {
    DispatchQueue.main.async {
        unmap()
        exit(0)
    }
}
let registered = DispatchSemaphore(value: 0)
terminate.setRegistrationHandler { registered.signal() }
terminate.resume()
registered.wait()
signal(SIGTERM, SIG_IGN)

register(shiftKey, shiftedID, &shifted)
func observeChanges() {
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
        stopRecheck()
        Mapping.apply(on: false)
    }
    workspace.addObserver(
        forName: NSWorkspace.sessionDidBecomeActiveNotification, object: nil, queue: .main
    ) { _ in
        sessionActive = true
        remap()
        remapLater()
        startRecheck()
    }
    NotificationCenter.default.addObserver(
        forName: NSApplication.willTerminateNotification, object: nil, queue: .main
    ) { _ in unmap() }
}
Mapping.start(
    readSession: { CGSessionCopyCurrentDictionary() as? [String: Any] },
    observe: observeChanges,
    remap: {
        sessionActive = true
        remap()
    })
follow(NSWorkspace.shared.frontmostApplication)
remapLater()
startRecheck()

NSApplication.shared.setActivationPolicy(.prohibited)
NSApplication.shared.run()
