import hashlib
import io
import pathlib
import struct
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
PACKAGER = ROOT / "scripts/package_flow_agent.py"
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


class DownloadBundleTest(unittest.TestCase):
    def binaries(self, root, platform):
        directory = root / "binaries"
        directory.mkdir()
        if platform == "ubuntu-24.04-x86_64":
            header = b"\x7fELF\x02\x01\x01" + bytes(11) + struct.pack("<H", 62)
        else:
            header = struct.pack("<II", 0xFEEDFACF, 0x0100000C)
        for name in ("flow", "flow-executor"):
            (directory / name).write_bytes(header + bytes(64) + name.encode())
        return directory

    def package(self, binaries, output, platform):
        return subprocess.run(
            [sys.executable, str(PACKAGER), "--binaries", str(binaries),
             "--output", str(output), "--platform", platform],
            capture_output=True, text=True, timeout=15,
        )

    def test_platform_archives_cover_final_installer_and_program_bytes(self):
        for platform in ("ubuntu-24.04-x86_64", "macos-26-aarch64"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                binaries = self.binaries(root, platform)
                output = root / "download"
                result = self.package(binaries, output, platform)
                self.assertEqual(result.returncode, 0, result.stderr)
                name = f"flow-agent-{VERSION}-{platform}"
                archive = output / f"{name}.tar.gz"
                final_bytes = archive.read_bytes()
                checksum = (output / "SHA256SUMS").read_text()
                self.assertEqual(checksum, f"{hashlib.sha256(final_bytes).hexdigest()}  {archive.name}\n")
                self.assertEqual(sorted(path.name for path in output.iterdir()),
                                 ["SHA256SUMS", archive.name])
                expected = {"install.sh": (ROOT / "install/install.sh").read_bytes(),
                            "flow": (binaries / "flow").read_bytes(),
                            "flow-executor": (binaries / "flow-executor").read_bytes(),
                            "bundle-info": f"{VERSION}\n{platform}\n".encode(),
                            "LICENSE": (ROOT / "LICENSE").read_bytes()}
                with tarfile.open(fileobj=io.BytesIO(final_bytes), mode="r:gz") as bundle:
                    self.assertEqual(bundle.getnames(), [name, *(f"{name}/{key}" for key in expected)])
                    for leaf, contents in expected.items():
                        entry = bundle.getmember(f"{name}/{leaf}")
                        self.assertTrue(entry.isfile())
                        self.assertEqual(entry.mode, 0o644 if leaf in ("bundle-info", "LICENSE") else 0o755)
                        self.assertEqual(bundle.extractfile(entry).read(), contents)
                repeated = root / "repeated"
                self.assertEqual(self.package(binaries, repeated, platform).returncode, 0)
                self.assertEqual((repeated / archive.name).read_bytes(), final_bytes)
                # A second invocation must not silently replace a prepared release.
                rejected = self.package(binaries, output, platform)
                self.assertNotEqual(rejected.returncode, 0)
                self.assertEqual(archive.read_bytes(), final_bytes)
                self.assertEqual((output / "SHA256SUMS").read_text(), checksum)

    def test_missing_or_wrong_architecture_program_does_not_produce_a_download(self):
        platform = "ubuntu-24.04-x86_64"
        for fault in ("missing", "wrong-architecture", "not-executable-format"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                binaries = self.binaries(root, platform)
                executor = binaries / "flow-executor"
                if fault == "missing":
                    executor.unlink()
                elif fault == "wrong-architecture":
                    executor.write_bytes(struct.pack("<II", 0xFEEDFACF, 0x0100000C) + bytes(64))
                else:
                    executor.write_bytes(b"not a native executable")
                output = root / "download"
                result = self.package(binaries, output, platform)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists(), result.stderr)


if __name__ == "__main__":
    unittest.main()
