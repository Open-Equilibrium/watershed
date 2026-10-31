import Foundation
import IOKit.hid
import IOKit.hidsystem

/// Caps Lock as the Liquid key: every keyboard sends F18 for it while the key is on.
enum Mapping {
    static let capsLock: UInt64 = 0x7_0000_0039
    static let f18: UInt64 = 0x7_0000_006D
    static let source = "HIDKeyboardModifierMappingSrc"
    static let destination = "HIDKeyboardModifierMappingDst"

    /// A keyboard's mappings with the Liquid key on or off; other mappings stay. On replaces any
    /// other Caps Lock mapping, off removes only Liquid's.
    static func merged(_ current: [[String: Any]], on: Bool) -> [[String: Any]] {
        func number(_ entry: [String: Any], _ key: String) -> UInt64? {
            (entry[key] as? NSNumber)?.uint64Value
        }
        let kept = current.filter { entry in
            guard number(entry, source) == capsLock else { return true }
            return !on && number(entry, destination) != f18
        }
        return on ? kept + [[source: capsLock, destination: f18]] : kept
    }

    /// Applies the Liquid key to every keyboard attached now; needs no permission. Returns
    /// whether macOS listed the keyboards and each one took it.
    @discardableResult
    static func apply(on: Bool) -> Bool {
        let key = "UserKeyMapping" as CFString
        let client = IOHIDEventSystemClientCreateSimpleClient(kCFAllocatorDefault)
        guard let services = IOHIDEventSystemClientCopyServices(client) as? [IOHIDServiceClient]
        else { return false }
        let keyboards = services.filter { service in
            IOHIDServiceClientConformsTo(
                service, UInt32(kHIDPage_GenericDesktop), UInt32(kHIDUsage_GD_Keyboard)) != 0
        }
        return apply(
            on: on, to: keyboards,
            read: { IOHIDServiceClientCopyProperty($0, key) as? [[String: Any]] ?? [] },
            write: { IOHIDServiceClientSetProperty($0, key, $1 as CFArray) })
    }

    /// Applies the Liquid key to each keyboard, also after one refuses it; returns whether all
    /// took it.
    static func apply<Keyboard>(
        on: Bool, to keyboards: [Keyboard], read: (Keyboard) -> [[String: Any]],
        write: (Keyboard, [[String: Any]]) -> Bool
    ) -> Bool {
        !keyboards.map { write($0, merged(read($0), on: on)) }.contains(false)
    }
}

/// A press of the Liquid key: a tap, which opens Liquid, when released soon with no other key
/// pressed meanwhile. Without permission macOS reports only the latest press of any key,
/// `lastKeyDown`, so each of the key's own repeats checks for another key since the press
/// before it, as the release does.
struct Press {
    let pressed: TimeInterval
    /// The key's latest press, its own repeats included.
    private var lastPress: TimeInterval
    private var otherKey = false

    init(at time: TimeInterval) {
        pressed = time
        lastPress = time
    }

    /// The key's own repeat at `time`.
    mutating func repeated(at time: TimeInterval, lastKeyDown: TimeInterval) {
        otherKey = otherKey || otherKeyDown(lastKeyDown)
        lastPress = time
    }

    /// Whether a release at `time` ends a tap.
    func isTap(released time: TimeInterval, lastKeyDown: TimeInterval) -> Bool {
        time - pressed < 0.6 && !otherKey && !otherKeyDown(lastKeyDown)
    }

    /// The Liquid key's own press lands a moment after the hotkey reports it.
    private func otherKeyDown(_ lastKeyDown: TimeInterval) -> Bool {
        lastKeyDown - lastPress >= 0.05
    }
}
