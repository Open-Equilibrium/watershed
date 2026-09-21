import json
import unittest

from scripts.m12_native import linux_fault_filter, validate_probe


def filter_result(program, syscall, argument=0):
    accumulator = 0
    index = 0
    while index < len(program):
        code, yes, no, value = program[index]
        if code == 0x20:
            accumulator = {0: syscall, 16: argument}[value]
        elif code == 0x15:
            index += yes if accumulator == value else no
        elif code == 0x45:
            index += yes if accumulator & value else no
        elif code == 0x06:
            return value
        else:
            raise AssertionError(f"unexpected BPF instruction: {code}")
        index += 1
    raise AssertionError("fault filter did not return a decision")


class ReadinessContractTest(unittest.TestCase):
    def test_namespace_fault_denies_namespace_creation_not_ordinary_children(self):
        program = linux_fault_filter("user-namespace")
        for syscall, argument in ((272, 0x10000000), (56, 0x10000011)):
            self.assertEqual(filter_result(program, syscall, argument), 0x50001)
        self.assertEqual(filter_result(program, 435), 0x50026)  # clone3 -> ENOSYS
        for syscall, argument in ((56, 17), (59, 0), (157, 22), (317, 1)):
            self.assertEqual(filter_result(program, syscall, argument), 0x7FFF0000)

    def test_seccomp_fault_denies_filter_installation_not_process_startup(self):
        program = linux_fault_filter("seccomp")
        for syscall, argument in ((157, 22), (317, 1)):
            self.assertEqual(filter_result(program, syscall, argument), 0x50001)
        for syscall, argument in ((157, 38), (56, 17), (59, 0), (272, 0x10000000)):
            self.assertEqual(filter_result(program, syscall, argument), 0x7FFF0000)

    def test_probe_has_only_retained_native_metadata(self):
        probe = dict(backend="seatbelt", backend_version="26.0",
                     executor="flow-executor", executor_version="0.0.0",
                     platform="macos-26-aarch64", protocol_versions=["0"], ready=True,
                     schema="flow-executor-probe-v0",
                     supported_policy_features=["flow-owned-write-protection"])
        def validate(value, ready=True):
            return validate_probe(json.dumps(value), ready=ready,
                                  platform="macos-26-aarch64", backend="seatbelt")
        self.assertEqual(validate(probe), probe)
        for key, value in (("runtime_mounts", []), ("unknown", True),
                           ("supported_policy_features", ["static-self-reexec"]),
                           ("protocol_versions", ["1"]), ("executor", "other")):
            with self.subTest(key=key), self.assertRaises(AssertionError):
                validate({**probe, key: value})
        unavailable = {**probe, "ready": False, "supported_policy_features": []}
        self.assertEqual(validate(unavailable, ready=False), unavailable)
        with self.assertRaises(AssertionError):
            validate(unavailable)


if __name__ == "__main__":
    unittest.main()
