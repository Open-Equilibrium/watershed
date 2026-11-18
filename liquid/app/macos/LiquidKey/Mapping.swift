import CoreGraphics
import Foundation
import IOKit.hid
import IOKit.hidsystem

enum Mapping {
    static let capsLock: UInt64 = 0x7_0000_0039
    static let f18: UInt64 = 0x7_0000_006D
    static let source = "HIDKeyboardModifierMappingSrc"
    static let destination = "HIDKeyboardModifierMappingDst"
    private static let userKey = "UserKeyMapping" as CFString

    static func merged(_ current: [[String: Any]], on: Bool) -> [[String: Any]] {
        let kept = current.filter { entry in
            guard number(entry, source) == capsLock else { return true }
            return !on && number(entry, destination) != f18
        }
        return on ? kept + [[source: capsLock, destination: f18]] : kept
    }

    static func needsReapply(_ current: [[String: Any]], active: Bool) -> Bool {
        let holds = current.contains { entry in
            number(entry, source) == capsLock && number(entry, destination) == f18
        }
        return active && !holds
    }

    private static func number(_ entry: [String: Any], _ key: String) -> UInt64? {
        (entry[key] as? NSNumber)?.uint64Value
    }

    static func givesBack(mapped: Bool, active: Bool) -> Bool {
        mapped && active
    }

    static func initiallyActive(session: [String: Any]?) -> Bool {
        (session?[kCGSessionOnConsoleKey as String] as? Bool) ?? false
    }

    static func start(
        readSession: () -> [String: Any]?, observe: () -> Void, remap: () -> Void
    ) {
        observe()
        if initiallyActive(session: readSession()) { remap() }
    }

    @discardableResult
    static func apply(on: Bool) -> Bool {
        guard let keyboards = attachedKeyboards() else { return false }
        return apply(on: on, to: keyboards, read: readMappings, write: writeMappings)
    }

    @discardableResult
    static func reapply(active: Bool) -> Bool {
        guard let keyboards = attachedKeyboards() else { return false }
        return reapply(active: active, to: keyboards, read: readMappings, write: writeMappings)
    }

    @discardableResult
    static func reapply<Keyboard>(
        active: Bool, to keyboards: [Keyboard], read: (Keyboard) -> [[String: Any]],
        write: (Keyboard, [[String: Any]]) -> Bool
    ) -> Bool {
        apply(
            on: true, to: keyboards.filter { needsReapply(read($0), active: active) },
            read: read, write: write)
    }

    private static func attachedKeyboards() -> [IOHIDServiceClient]? {
        let client = IOHIDEventSystemClientCreateSimpleClient(kCFAllocatorDefault)
        guard let services = IOHIDEventSystemClientCopyServices(client) as? [IOHIDServiceClient]
        else { return nil }
        return services.filter { service in
            IOHIDServiceClientConformsTo(
                service, UInt32(kHIDPage_GenericDesktop), UInt32(kHIDUsage_GD_Keyboard)) != 0
        }
    }

    private static func readMappings(_ keyboard: IOHIDServiceClient) -> [[String: Any]] {
        IOHIDServiceClientCopyProperty(keyboard, userKey) as? [[String: Any]] ?? []
    }

    private static func writeMappings(
        _ keyboard: IOHIDServiceClient, _ mappings: [[String: Any]]
    ) -> Bool {
        IOHIDServiceClientSetProperty(keyboard, userKey, mappings as CFArray)
    }

    static func apply<Keyboard>(
        on: Bool, to keyboards: [Keyboard], read: (Keyboard) -> [[String: Any]],
        write: (Keyboard, [[String: Any]]) -> Bool
    ) -> Bool {
        !keyboards.map { write($0, merged(read($0), on: on)) }.contains(false)
    }
}

struct Press {
    let pressed: TimeInterval
    private var lastPress: TimeInterval
    private var otherKey = false

    init(at time: TimeInterval) {
        pressed = time
        lastPress = time
    }

    mutating func repeated(at time: TimeInterval, lastKeyDown: TimeInterval) {
        otherKey = otherKey || otherKeyDown(lastKeyDown)
        lastPress = time
    }

    func isTap(released time: TimeInterval, lastKeyDown: TimeInterval) -> Bool {
        time - pressed < 0.6 && !otherKey && !otherKeyDown(lastKeyDown)
    }

    private func otherKeyDown(_ lastKeyDown: TimeInterval) -> Bool {
        lastKeyDown - lastPress >= 0.05
    }
}
