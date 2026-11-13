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

var keyboards = [[entry(Mapping.capsLock, Mapping.f18)], [entry(Mapping.capsLock, Mapping.f18)]]
func turnOff(refusing: Int?) -> Bool {
    Mapping.apply(on: false, to: Array(keyboards.indices), read: { keyboards[$0] }) {
        keyboard, mappings in
        guard keyboard != refusing else { return false }
        keyboards[keyboard] = mappings
        return true
    }
}
check(
    !turnOff(refusing: 0) && keyboards[1].isEmpty,
    "turning it off reports a keyboard that refuses and still turns the others off")
check(
    turnOff(refusing: nil) && keyboards.allSatisfy(\.isEmpty),
    "turning it off succeeds once every keyboard took it")

// Another session's helper may give the remapping back after this one set it. Only an active
// helper sets it again, and only on a keyboard that lost it.
var lost = [[entry(control, command)], [entry(Mapping.capsLock, Mapping.f18)]]
var writes = 0
func recheck(active: Bool) {
    Mapping.reapply(active: active, to: Array(lost.indices), read: { lost[$0] }) {
        keyboard, mappings in
        writes += 1
        lost[keyboard] = mappings
        return true
    }
}
recheck(active: false)
check(
    writes == 0 && pairs(lost[0]) == pairs([entry(control, command)]),
    "an inactive helper does not re-apply the Liquid key")
recheck(active: true)
check(
    writes == 1
        && pairs(lost[0]) == pairs([entry(control, command), entry(Mapping.capsLock, Mapping.f18)]),
    "an active helper re-applies a mapping another session removed and keeps the others")

/// A Liquid key press at 10 s with its own `repeats`, each with the latest key press of any key
/// then, released at `released`.
func tap(
    _ repeats: [(at: TimeInterval, lastKeyDown: TimeInterval)] = [], released: TimeInterval,
    lastKeyDown: TimeInterval
) -> Bool {
    var press = Press(at: 10)
    for next in repeats { press.repeated(at: next.at, lastKeyDown: next.lastKeyDown) }
    return press.isTap(released: released, lastKeyDown: lastKeyDown)
}
check(tap(released: 10.3, lastKeyDown: 10.001), "a short press is a tap")
check(!tap(released: 10.75, lastKeyDown: 10.001), "a hold is no tap")
check(!tap(released: 10.3, lastKeyDown: 10.15), "another key pressed meanwhile makes it no tap")
check(
    tap([(10.5, 10.001)], released: 10.55, lastKeyDown: 10.501),
    "the key's own repeat is no other key")
check(
    !tap([(10.5, 10.2)], released: 10.55, lastKeyDown: 10.501),
    "another key before the key's own repeat makes it no tap")

check(
    !Mapping.givesBack(mapped: false, active: true),
    "a helper that never mapped gives nothing back")
check(
    Mapping.givesBack(mapped: true, active: true),
    "a helper that mapped gives its remapping back while its session is in front")
check(
    !Mapping.givesBack(mapped: true, active: false),
    "a helper whose session left gives nothing back")

if failures > 0 { exit(1) }
print("Liquid key helper checks passed")
