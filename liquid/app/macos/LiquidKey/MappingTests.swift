import Foundation

// Checks the helper's pure decisions; `test.sh` runs it on the macOS CI host.

var failures = 0
func check(_ condition: Bool, _ what: String) {
    if !condition {
        failures += 1
        print("FAIL: \(what)")
    }
}

func entry(_ source: UInt64, _ destination: UInt64) -> [String: Any] {
    [Mapping.source: NSNumber(value: source), Mapping.destination: NSNumber(value: destination)]
}

func pairs(_ mappings: [[String: Any]]) -> [[UInt64]] {
    mappings.map { entry in
        [Mapping.source, Mapping.destination].map { (entry[$0] as! NSNumber).uint64Value }
    }
}

let control: UInt64 = 0x7_0000_00E0
let command: UInt64 = 0x7_0000_00E3
let escape: UInt64 = 0x7_0000_0029
let swapped = [entry(control, command), entry(command, control)]

check(
    pairs(Mapping.merged(swapped, on: true))
        == pairs(swapped + [entry(Mapping.capsLock, Mapping.f18)]),
    "turning the Liquid key on keeps other mappings")
check(
    pairs(Mapping.merged([entry(Mapping.capsLock, escape)], on: true))
        == [[Mapping.capsLock, Mapping.f18]],
    "turning it on replaces another Caps Lock mapping")
check(
    pairs(Mapping.merged(swapped + [entry(Mapping.capsLock, Mapping.f18)], on: false))
        == pairs(swapped),
    "turning it off removes only the Liquid key")
check(
    pairs(Mapping.merged([entry(Mapping.capsLock, escape)], on: false))
        == [[Mapping.capsLock, escape]],
    "turning it off keeps a Caps Lock mapping that is not Liquid's")
check(pairs(Mapping.merged([], on: true)).count == 1, "turning it on adds the Liquid key")
check(
    pairs(Mapping.merged(Mapping.merged([], on: true), on: true)).count == 1,
    "turning it on again keeps one mapping")
check(isTap(pressed: 10, released: 10.3), "a short press is a tap")
check(!isTap(pressed: 10, released: 10.75), "a hold is no tap")

if failures > 0 { exit(1) }
print("Liquid key helper checks passed")
