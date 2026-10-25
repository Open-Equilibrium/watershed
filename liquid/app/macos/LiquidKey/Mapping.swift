import Foundation
import IOKit.hid
import IOKit.hidsystem

/// Caps Lock as the Liquid key (ADR-0205): every keyboard sends F18 for it while the key is on.
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

    /// Applies the Liquid key to every keyboard attached now; needs no permission.
    static func apply(on: Bool) {
        let key = "UserKeyMapping" as CFString
        let client = IOHIDEventSystemClientCreateSimpleClient(kCFAllocatorDefault)
        guard let services = IOHIDEventSystemClientCopyServices(client) as? [IOHIDServiceClient]
        else { return }
        for service in services
        where IOHIDServiceClientConformsTo(
            service, UInt32(kHIDPage_GenericDesktop), UInt32(kHIDUsage_GD_Keyboard)) != 0
        {
            let current = IOHIDServiceClientCopyProperty(service, key) as? [[String: Any]] ?? []
            IOHIDServiceClientSetProperty(service, key, merged(current, on: on) as CFArray)
        }
    }
}

/// A press and release of the Liquid key short enough to open Liquid, not a hold.
func isTap(pressed: TimeInterval, released: TimeInterval) -> Bool {
    released - pressed < 0.6
}
