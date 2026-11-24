import hashlib
import os
import pathlib
import platform
import re
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
import zipfile


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
    if not root.exists():
        return {}
    return {str(path.relative_to(root)): path.read_bytes() if path.is_file() else None
            for path in sorted(root.rglob("*"))}


def run_packager(root, target, output, t3_version=T3_VERSION, t3=None):
    meta = write(root / "fixtures" / "meta", b"synthetic meta program", 0o755)
    if t3 is None:
        t3 = root / "fixtures" / f"t3-{t3_version}-{target}"
        write(t3 / "t3", b"synthetic T3 program", 0o755)
        (t3 / "helper").symlink_to("t3")
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
        folder = self.root / f"bundle-{generation}"
        write(folder / "bin" / "meta",
              f"#!/bin/sh\n# {generation}\nprintf '%s %s\\n' \"$0\" \"$*\" >> {shlex.quote(str(self.log))}\n",
              0o755)
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
        result = self.install(bundle)
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

    def test_failed_service_install_keeps_the_files_and_exits_non_zero(self):
        prefix = self.root / "prefix"
        bundle = self.bundle("0.0.1", HOST, "0.1.9", "first")
        write(bundle / "bin" / "meta",
              f"#!/bin/sh\necho \"$*\" >> {shlex.quote(str(self.log))}\nexit 1\n", 0o755)
        result = self.install(bundle, "--prefix", str(prefix))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(str(prefix), result.stderr)
        self.assertIn("did not start", result.stderr)
        self.assertIn("--no-service", result.stderr)
        self.assertEqual(self.log_lines(), ["service install"])
        self.assertTrue((prefix / "bin" / "meta").is_file())

    def test_refused_installs_change_nothing(self):
        absolute = str(self.root / "prefix")
        good = self.bundle("0.0.1", HOST, "0.1.9", "good")
        other = self.bundle("0.0.1", OTHER, "0.1.9", "other")
        cases = {
            "relative prefix": (good, ["--prefix", "relative/prefix"]),
            "newline in prefix": (good, ["--prefix", absolute + "\nx"]),
            "other platform": (other, ["--prefix", absolute]),
            "data folder is the prefix": (good, ["--prefix", str(self.home / ".meta-harness")]),
        }
        for name, (bundle, args) in cases.items():
            with self.subTest(case=name):
                before = snapshot(self.root)
                result = self.install(bundle, *args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("meta-install:", result.stderr)
                self.assertEqual(snapshot(self.root), before)

    @unittest.skipUnless(shutil.which("node"), "the packager runs on Node, which is not on PATH")
    def test_a_packaged_download_installs_an_executable_t3_with_its_relative_links(self):
        output = self.root / "download"
        result = run_packager(self.root, HOST, output)
        self.assertEqual(result.returncode, 0, result.stderr)
        unpacked = self.root / "unpacked"
        with tarfile.open(output / f"meta-harness-{META_VERSION}-{HOST}.tar.gz") as archive:
            archive.extractall(unpacked, filter="data")
        bundle = unpacked / f"meta-harness-{META_VERSION}-{HOST}"
        t3_name = f"t3-{T3_VERSION}-{HOST}"
        result = self.install(bundle, "--prefix", str(self.root / "prefix"), "--no-service")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log_lines(), [])
        installed = self.root / "prefix" / "lib" / "meta-harness" / t3_name
        self.assertTrue(os.access(installed / "t3", os.X_OK))
        self.assertEqual(os.readlink(installed / "helper"), "t3")

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
                    links = {member.name.removeprefix(f"{name}/"): member.linkname
                             for member in bundle.getmembers() if member.issym()}
                    self.assertEqual(links, {f"lib/meta-harness/t3-{T3_VERSION}-{target}/helper": "t3"})

    def test_a_windows_download_is_a_zip_with_meta_exe_and_no_shell_installer(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            target = "win32-x64"
            t3 = root / "fixtures" / f"t3-{T3_VERSION}-{target}"
            write(t3 / "t3.exe", b"synthetic T3 program")
            write(t3 / "client" / "index.html", b"synthetic client")
            output = root / "download"
            result = run_packager(root, target, output, t3=t3)
            self.assertEqual(result.returncode, 0, result.stderr)
            name = f"meta-harness-{META_VERSION}-{target}"
            archive = output / f"{name}.zip"
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            self.assertEqual((output / "SHA256SUMS").read_text(encoding="ascii"),
                             f"{digest}  {archive.name}\n")
            expected = {
                "bin/meta.exe": b"synthetic meta program",
                f"lib/meta-harness/t3-{T3_VERSION}-{target}/t3.exe": b"synthetic T3 program",
                f"lib/meta-harness/t3-{T3_VERSION}-{target}/client/index.html": b"synthetic client",
                "bundle-info": f"{META_VERSION}\n{target}\n".encode("ascii"),
                "LICENSE": (ROOT / "LICENSE").read_bytes(),
                "LICENSE-t3code": (ROOT / "meta-harness" / "LICENSE-t3code").read_bytes(),
                "THIRD_PARTY_NOTICES.md": (ROOT / "THIRD_PARTY_NOTICES.md").read_bytes(),
            }
            with zipfile.ZipFile(archive) as bundle:
                self.assertIsNone(bundle.testzip())
                files = {info.filename.removeprefix(f"{name}/"): info for info in bundle.infolist()}
                self.assertEqual(set(files), set(expected))
                for relative, contents in expected.items():
                    self.assertEqual(bundle.read(files[relative]), contents, relative)

    def test_a_windows_download_refuses_a_t3_folder_with_a_symlink_inside(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            target = "win32-x64"
            t3 = root / "fixtures" / f"t3-{T3_VERSION}-{target}"
            write(t3 / "t3.exe", b"synthetic T3 program")
            (t3 / "helper").symlink_to("t3.exe")
            output = root / "download"
            result = run_packager(root, target, output, t3=t3)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("neither a file nor a folder", result.stderr)
            self.assertFalse(output.exists(), result.stderr)

    def test_a_symlinked_t3_folder_is_packed_as_the_folder_it_names(self):
        for target in PLATFORMS:
            with self.subTest(platform=target), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                write(root / "real" / "t3", b"synthetic T3 program", 0o755)
                link = root / "links" / f"t3-{T3_VERSION}-{target}"
                link.parent.mkdir()
                link.symlink_to(root / "real")
                output = root / "download"
                result = run_packager(root, target, output, t3=link)
                self.assertEqual(result.returncode, 0, result.stderr)
                name = f"meta-harness-{META_VERSION}-{target}"
                with tarfile.open(output / f"{name}.tar.gz", "r:gz") as bundle:
                    member = bundle.getmember(f"{name}/lib/meta-harness/t3-{T3_VERSION}-{target}/t3")
                    self.assertTrue(member.isfile())
                    self.assertEqual(bundle.extractfile(member).read(), b"synthetic T3 program")

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
