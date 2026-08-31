import os
import pathlib
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
INSTALLER = ROOT / "install" / "install.sh"
PROBE_DOCUMENT = (
    '{"backend":"bubblewrap-seccomp","backend_version":"test",'
    '"executor":"flow-executor","executor_version":"0.0.0",'
    '"platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],'
    '"ready":true,"runtime_mounts":[],"schema":"flow-executor-probe-v0",'
    '"supported_policy_features":["static-self-reexec"]}'
)


@unittest.skipUnless(
    sys.platform.startswith("linux")
    and pathlib.Path("/bin/sh").exists()
    and pathlib.Path("/usr/bin/setsid").exists(),
    "requires Linux /bin/sh and /usr/bin/setsid",
)
class PrefixInstallerTest(unittest.TestCase):
    def bundle(self, root: pathlib.Path) -> pathlib.Path:
        bundle = root / "bundle"
        bundle.mkdir(mode=0o755, parents=True)
        shutil.copy2(INSTALLER, bundle / "install.sh")
        flow = bundle / "flow"
        flow.write_text(
            "#!/bin/sh\n"
            "test \"$1 $2\" = \"executor check\" || exit 64\n"
            "test -n \"$XDG_CONFIG_HOME\" || exit 65\n"
            "test \"$HOME\" = \"$XDG_CONFIG_HOME\" || exit 65\n"
            "test ! -e \"$XDG_CONFIG_HOME/flow-agent/executor.json\" || exit 65\n"
            "probe=$(\"${0%/*}/flow-executor\" --probe) || exit 65\n"
            f"expected='{PROBE_DOCUMENT}'\n"
            "test \"$probe\" = \"$expected\" || exit 65\n",
            encoding="utf-8",
        )
        executor = bundle / "flow-executor"
        executor.write_text(
            "#!/bin/sh\n"
            "test \"$#\" -eq 1 && test \"$1\" = \"--probe\" || exit 64\n"
            f"printf '%s\\n' '{PROBE_DOCUMENT}'\n",
            encoding="utf-8",
        )
        for path in (bundle / "install.sh", flow, executor):
            path.chmod(0o755)
        return bundle

    def install(
        self, bundle: pathlib.Path, prefix: pathlib.Path, *args: str, trace: bool = False
    ):
        unrelated_cwd = prefix.parent / "unrelated-cwd"
        unrelated_cwd.mkdir(exist_ok=True)
        return subprocess.run(
            [
                "/bin/sh",
                *(["-x"] if trace else []),
                str(bundle / "install.sh"),
                "--prefix",
                str(prefix),
                *args,
            ],
            cwd=unrelated_cwd,
            env={"PATH": ""},
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_standard_and_opt_out_install_from_any_cwd_with_empty_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)

            standard = root / "standard"
            installed = self.install(bundle, standard)
            self.assertEqual(installed.returncode, 0, installed.stderr)
            self.assertTrue((standard / "bin" / "flow").is_file())
            self.assertTrue((standard / "bin" / "flow-executor").is_file())

            opt_out = root / "opt-out"
            installed = self.install(bundle, opt_out, "--no-default-executor")
            self.assertEqual(installed.returncode, 0, installed.stderr)
            self.assertTrue((opt_out / "bin" / "flow").is_file())
            self.assertFalse((opt_out / "bin" / "flow-executor").exists())

    def test_successful_readiness_cleans_while_its_process_group_is_reserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            installed = self.install(self.bundle(root), root / "prefix", trace=True)

            self.assertEqual(installed.returncode, 0, installed.stderr)
            trace = installed.stderr.decode("utf-8", errors="replace")
            wait_index = trace.index("+ wait ")
            self.assertLess(trace.index("/bin/kill"), wait_index)
            self.assertNotIn("/bin/kill", trace[wait_index:])
            self.assertNotIn("/bin/kill -KILL", trace)

    def test_existing_targets_and_unsafe_bundle_inputs_fail_without_upgrade(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            prefix = root / "prefix"
            first = self.install(bundle, prefix, "--no-default-executor")
            self.assertEqual(first.returncode, 0, first.stderr)

            repeated = self.install(bundle, prefix, "--no-default-executor")
            self.assertNotEqual(repeated.returncode, 0)
            self.assertIn(b"existing installation", repeated.stderr)

            unsafe_bundle = self.bundle(root / "unsafe")
            executor = unsafe_bundle / "flow-executor"
            executor.unlink()
            executor.symlink_to(unsafe_bundle / "flow")
            unsafe_prefix = root / "unsafe-prefix"
            rejected = self.install(unsafe_bundle, unsafe_prefix)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((unsafe_prefix / "bin" / "flow").exists())

    def test_installed_files_are_regular_executable_siblings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            prefix = root / "prefix"

            installed = self.install(bundle, prefix)

            self.assertEqual(installed.returncode, 0, installed.stderr)
            for name in ("flow", "flow-executor"):
                path = prefix / "bin" / name
                metadata = path.lstat()
                self.assertTrue(stat.S_ISREG(metadata.st_mode))
                self.assertEqual(metadata.st_nlink, 1)
                self.assertEqual(metadata.st_uid, os.geteuid())
                self.assertEqual(metadata.st_mode & 0o022, 0)

    def test_failed_readiness_rolls_back_every_published_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            (bundle / "flow").write_text(
                "#!/bin/sh\n"
                ": > \"$XDG_CONFIG_HOME/readiness-state\"\n"
                "exit 65\n",
                encoding="utf-8",
            )
            (bundle / "flow").chmod(0o755)
            prefix = root / "prefix"

            installed = self.install(bundle, prefix)

            self.assertNotEqual(installed.returncode, 0)
            self.assertIn(b"failed readiness", installed.stderr)
            self.assertEqual(list((prefix / "bin").iterdir()), [])

    def test_signal_at_each_publication_boundary_rolls_back(self):
        boundaries = (
            (
                '/bin/ln -- "$flow_stage" "$flow_target" || fail \'cannot publish flow\'\n',
                "--no-default-executor",
            ),
            (
                '/bin/ln -- "$executor_stage" "$executor_target" || fail \'cannot publish flow-executor\'\n',
                None,
            ),
        )
        for publication, opt_out in boundaries:
            with self.subTest(publication=publication), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                bundle = self.bundle(root)
                installer = bundle / "install.sh"
                source = installer.read_text(encoding="utf-8")
                self.assertEqual(source.count(publication), 1)
                installer.write_text(
                    source.replace(
                        publication,
                        publication + '/bin/kill -TERM "$$"\n',
                    ),
                    encoding="utf-8",
                )
                installer.chmod(0o755)
                prefix = root / "prefix"

                args = (opt_out,) if opt_out else ()
                installed = self.install(bundle, prefix, *args)

                self.assertEqual(installed.returncode, 128 + signal.SIGTERM, installed.stderr)
                self.assertEqual(list((prefix / "bin").iterdir()), [])

    def test_signal_during_readiness_terminates_descendants_and_rolls_back(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            installer = bundle / "install.sh"
            source = installer.read_text(encoding="utf-8")
            self.assertEqual(source.count("/usr/bin/pgrep"), 1)
            installer.write_text(
                source.replace("/usr/bin/pgrep", "/missing/pgrep"),
                encoding="utf-8",
            )
            marker = root / "readiness-started"
            (bundle / "flow").write_text(
                "#!/bin/sh\n"
                "trap 'exit 1' HUP INT TERM\n"
                "(\n"
                "    trap '' HUP INT TERM\n"
                "    while :; do /bin/sleep 30; done\n"
                ") &\n"
                "descendant=$!\n"
                f"printf '%s\\n' \"$descendant\" > '{marker}'\n"
                "wait \"$descendant\"\n",
                encoding="utf-8",
            )
            (bundle / "flow").chmod(0o755)
            prefix = root / "prefix"
            unrelated_cwd = root / "unrelated-cwd"
            unrelated_cwd.mkdir()
            process = subprocess.Popen(
                ["/bin/sh", str(bundle / "install.sh"), "--prefix", str(prefix)],
                cwd=unrelated_cwd,
                env={"PATH": ""},
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            descendant = None
            for _ in range(200):
                if marker.exists() and marker.read_text(encoding="utf-8").strip():
                    descendant = int(marker.read_text(encoding="utf-8").strip())
                    break
                if process.poll() is not None:
                    self.fail(process.stderr.read().decode("utf-8", errors="replace"))
                time.sleep(0.01)
            else:
                process.kill()
                self.fail("installer did not enter readiness")

            try:
                process.send_signal(signal.SIGTERM)
                _, stderr = process.communicate(timeout=5)

                self.assertEqual(process.returncode, 128 + signal.SIGTERM, stderr)
                self.assertEqual(list((prefix / "bin").iterdir()), [])
                for _ in range(200):
                    try:
                        os.kill(descendant, 0)
                    except ProcessLookupError:
                        break
                    time.sleep(0.01)
                else:
                    self.fail(f"readiness descendant {descendant} survived cleanup")
            finally:
                try:
                    os.kill(descendant, signal.SIGKILL)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    unittest.main()
