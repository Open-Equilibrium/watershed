import hashlib
import os
import pathlib
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "meta-harness" / "install.sh"
PACKAGER = ROOT / "scripts" / "package-meta-harness.mjs"
META_VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
T3_VERSION = re.search(r"^version (\S+)$", (ROOT / "meta-harness" / "t3.lock").read_text(encoding="utf-8"),
                       re.MULTILINE).group(1)
PLATFORMS = ("darwin-arm64", "linux-x64")
HOST = {("Darwin", "arm64"): "darwin-arm64", ("Linux", "x86_64"): "linux-x64"}.get(
    (platform.system(), platform.machine()))
OTHER = "linux-x64" if HOST == "darwin-arm64" else "darwin-arm64"
LICENSES = ("LICENSE", "LICENSE-t3code", "THIRD_PARTY_NOTICES.md")
ROOT_USER = hasattr(os, "geteuid") and os.geteuid() == 0


def write(path, content, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content if isinstance(content, bytes) else content.encode("utf-8"))
    path.chmod(mode)
    return path


def snapshot(root):
    """Every path under root with its file bytes (None for a folder), to prove nothing changed."""
    if not root.exists():
        return {}
    return {str(path.relative_to(root)): path.read_bytes() if path.is_file() else None
            for path in sorted(root.rglob("*"))}


def run_packager(root, target, output, t3_version=T3_VERSION):
    meta = write(root / "fixtures" / "meta", b"synthetic meta program", 0o755)
    t3 = root / "fixtures" / f"t3-{t3_version}-{target}"
    write(t3 / "t3", b"synthetic T3 program", 0o755)
    return subprocess.run(
        ["node", str(PACKAGER), "--meta", str(meta), "--t3", str(t3), "--platform", target,
         "--output", str(output)],
        capture_output=True, text=True, timeout=60,
    )


@unittest.skipUnless(HOST, f"install.sh needs a Darwin arm64 or Linux x86_64 host, not "
                           f"{platform.system()} {platform.machine()}")
class InstallerTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.log = self.root / "service.log"

    def bundle(self, version, target, t3_version, generation):
        """An unpacked download laid out as the packager writes it; its meta logs its arguments."""
        folder = self.root / f"bundle-{generation}"
        write(folder / "bin" / "meta",
              f"#!/bin/sh\n# {generation}\nprintf '%s %s\\n' \"$0\" \"$*\" >> '{self.log}'\n", 0o755)
        write(folder / "lib" / "meta-harness" / f"t3-{t3_version}-{target}" / "t3",
              f"#!/bin/sh\n# {generation}\n", 0o755)
        write(folder / "install.sh", INSTALLER.read_bytes(), 0o755)
        write(folder / "bundle-info", f"{version}\n{target}\n")
        for name in LICENSES:
            write(folder / name, f"{name} {generation}\n")
        return folder

    def install(self, bundle, *args):
        return subprocess.run(
            ["sh", "install.sh", *args], cwd=bundle,
            env={"HOME": str(self.home), "PATH": os.environ["PATH"]},
            capture_output=True, text=True, timeout=60,
        )

    def log_lines(self):
        return self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []

    def test_fresh_install_places_every_file_and_starts_the_service(self):
        bundle = self.bundle("0.0.1", HOST, "0.1.9", "first")
        result = self.install(bundle)  # the default prefix under HOME
        self.assertEqual(result.returncode, 0, result.stderr)
        prefix = self.home / ".local" / "meta-harness"
        files = {"bin/meta": bundle / "bin" / "meta",
                 f"lib/meta-harness/t3-0.1.9-{HOST}/t3": bundle / "lib" / "meta-harness" / f"t3-0.1.9-{HOST}" / "t3"}
        files.update({f"share/meta-harness/{name}": bundle / name for name in LICENSES})
        for relative, source in files.items():
            self.assertEqual((prefix / relative).read_bytes(), source.read_bytes(), relative)
        self.assertTrue(os.access(prefix / "bin" / "meta", os.X_OK))
        self.assertEqual(self.log_lines(), [f"{prefix}/bin/meta service install"])
        self.assertIn(f'export PATH="{prefix}/bin:$PATH"', result.stdout)

    def test_newer_bundle_replaces_the_program_and_t3_and_restarts_the_service(self):
        prefix = self.root / "prefix"
        old = self.bundle("0.0.1", HOST, "0.1.8", "old")
        self.assertEqual(self.install(old, "--prefix", str(prefix)).returncode, 0)
        new = self.bundle("0.0.2", HOST, "0.1.9", "new")
        result = self.install(new, "--prefix", str(prefix))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((prefix / "bin" / "meta").read_bytes(), (new / "bin" / "meta").read_bytes())
        self.assertEqual(sorted(path.name for path in (prefix / "lib" / "meta-harness").iterdir()),
                         [f"t3-0.1.9-{HOST}"])
        self.assertEqual(self.log_lines(), [f"{prefix}/bin/meta service install"] * 2)

    def test_no_service_installs_without_running_the_program(self):
        prefix = self.root / "prefix"
        result = self.install(self.bundle("0.0.1", HOST, "0.1.9", "first"), "--prefix", str(prefix),
                              "--no-service")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((prefix / "bin" / "meta").is_file())
        self.assertEqual(self.log_lines(), [])

    def test_refused_installs_change_nothing(self):
        absolute = str(self.root / "prefix")
        good = self.bundle("0.0.1", HOST, "0.1.9", "good")
        other = self.bundle("0.0.1", OTHER, "0.1.9", "other")
        cases = {
            "relative prefix": (good, ["--prefix", "relative/prefix"]),
            "newline in prefix": (good, ["--prefix", absolute + "\nx"]),
            "other platform": (other, ["--prefix", absolute]),
        }
        for name, (bundle, args) in cases.items():
            with self.subTest(case=name):
                before = snapshot(self.root)
                result = self.install(bundle, *args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("meta-install:", result.stderr)
                self.assertEqual(snapshot(self.root), before)

    @unittest.skipIf(ROOT_USER, "root ignores directory permissions")
    def test_failed_install_keeps_the_previous_installation_and_leaves_no_temporary_file(self):
        prefix = self.root / "prefix"
        self.assertEqual(self.install(self.bundle("0.0.1", HOST, "0.1.8", "old"), "--prefix",
                                      str(prefix)).returncode, 0)
        bin_folder = prefix / "bin"
        bin_folder.chmod(0o555)
        self.addCleanup(bin_folder.chmod, 0o755)
        before = snapshot(prefix)
        result = self.install(self.bundle("0.0.2", HOST, "0.1.9", "new"), "--prefix", str(prefix))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Permission denied", result.stderr)
        self.assertEqual(snapshot(prefix), before)


@unittest.skipUnless(shutil.which("node"), "the packager runs on Node, which is not on PATH")
class PackagerTest(unittest.TestCase):
    def test_archive_unpacks_to_the_bundle_tree_and_its_checksum_matches(self):
        for target in PLATFORMS:
            with self.subTest(platform=target), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                output = root / "download"
                result = run_packager(root, target, output)
                self.assertEqual(result.returncode, 0, result.stderr)
                name = f"meta-harness-{META_VERSION}-{target}"
                archive = output / f"{name}.tar.gz"
                self.assertEqual(sorted(path.name for path in output.iterdir()),
                                 sorted([archive.name, "SHA256SUMS"]))
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                self.assertEqual((output / "SHA256SUMS").read_text(encoding="ascii"),
                                 f"{digest}  {archive.name}\n")
                expected = {
                    "bin/meta": b"synthetic meta program",
                    f"lib/meta-harness/t3-{T3_VERSION}-{target}/t3": b"synthetic T3 program",
                    "install.sh": INSTALLER.read_bytes(),
                    "bundle-info": f"{META_VERSION}\n{target}\n".encode("ascii"),
                    "LICENSE": (ROOT / "LICENSE").read_bytes(),
                    "LICENSE-t3code": (ROOT / "meta-harness" / "LICENSE-t3code").read_bytes(),
                    "THIRD_PARTY_NOTICES.md": (ROOT / "THIRD_PARTY_NOTICES.md").read_bytes(),
                }
                with tarfile.open(archive, "r:gz") as bundle:
                    files = {member.name.removeprefix(f"{name}/"): member
                             for member in bundle.getmembers() if member.isfile()}
                    self.assertEqual(set(files), set(expected))
                    for relative, contents in expected.items():
                        self.assertEqual(bundle.extractfile(files[relative]).read(), contents, relative)
                    for relative in ("bin/meta", "install.sh"):
                        self.assertEqual(files[relative].mode & 0o755, 0o755, relative)

    def test_refuses_an_existing_output_and_another_t3_version(self):
        for target in PLATFORMS:
            with self.subTest(platform=target), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                existing = write(root / "existing" / "SHA256SUMS", "prepared release\n")
                result = run_packager(root, target, existing.parent)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(existing.read_text(encoding="utf-8"), "prepared release\n")
                output = root / "other-version"
                result = run_packager(root, target, output, t3_version="0.0.1")
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists(), result.stderr)


if __name__ == "__main__":
    unittest.main()
