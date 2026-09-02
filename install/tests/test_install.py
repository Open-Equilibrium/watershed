import os
import pathlib
import shlex
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
    def test_root_readiness_uses_the_target_user_manager_without_enabling_linger(self):
        installer = INSTALLER.read_text(encoding="utf-8")

        self.assertIn('readiness_runtime_dir=/run/user/$readiness_owner', installer)
        self.assertIn('XDG_RUNTIME_DIR=$readiness_runtime_dir', installer)
        self.assertIn(
            'DBUS_SESSION_BUS_ADDRESS=unix:path=$readiness_runtime_dir/bus',
            installer,
        )
        self.assertNotIn("loginctl", installer)

    def test_help_succeeds_without_installing(self):
        expected = (
            "Usage: install.sh --prefix <absolute-prefix> [--no-default-executor]\n"
            "\n"
            "Install Flow Agent on Ubuntu 24.04 x64 from sibling bundle artifacts.\n"
            "\n"
            "Options:\n"
            "  --prefix <absolute-prefix>  Install into <absolute-prefix>/bin.\n"
            "  --no-default-executor       Install flow without the bundled Default Executor.\n"
            "  -h, --help                  Show this help.\n"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for flag in ("-h", "--help"):
                completed = subprocess.run(
                    ["/bin/sh", str(INSTALLER), flag],
                    cwd=root,
                    env={"PATH": ""},
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertEqual(completed.stdout, expected)
                self.assertEqual(completed.stderr, "")
                self.assertEqual(list(root.iterdir()), [])

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

    def install(self, bundle: pathlib.Path, prefix: pathlib.Path, *args: str):
        unrelated_cwd = prefix.parent / "unrelated-cwd"
        unrelated_cwd.mkdir(exist_ok=True)
        return subprocess.run(
            [
                "/bin/sh",
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

    def pause_installer_after_validation(
        self, bundle: pathlib.Path, marker: pathlib.Path, release: pathlib.Path
    ):
        installer = bundle / "install.sh"
        source = installer.read_text(encoding="utf-8")
        boundary = "trap 'signal_exit 143' TERM\n\n"
        self.assertEqual(source.count(boundary), 1)
        barrier = (
            f": > {shlex.quote(str(marker))}\n"
            f"while [ ! -e {shlex.quote(str(release))} ]; do /bin/sleep 0.01; done\n"
        )
        installer.write_text(
            source.replace(boundary, boundary + barrier),
            encoding="utf-8",
        )
        installer.chmod(0o755)

    def wait_for_installer_marker(
        self, process: subprocess.Popen, marker: pathlib.Path
    ):
        for _ in range(500):
            if marker.exists():
                return
            if process.poll() is not None:
                self.fail(process.stderr.read().decode("utf-8", errors="replace"))
            time.sleep(0.01)
        process.kill()
        self.fail("installer did not reach the post-validation boundary")

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
                "printf '%s\\n' 'executor_unavailable: Bubblewrap is unavailable' >&2\n"
                "exit 65\n",
                encoding="utf-8",
            )
            (bundle / "flow").chmod(0o755)
            prefix = root / "prefix"

            installed = self.install(bundle, prefix)

            self.assertNotEqual(installed.returncode, 0)
            self.assertIn(b"Bubblewrap is unavailable", installed.stderr)
            self.assertIn(b"failed readiness", installed.stderr)
            self.assertEqual(list((prefix / "bin").iterdir()), [])

    def test_replacing_validated_bundle_fails_without_running_substituted_flow(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            validated = root / "validated"
            release = root / "release"
            executed = root / "substituted-flow-ran"
            self.pause_installer_after_validation(bundle, validated, release)
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

            self.wait_for_installer_marker(process, validated)
            bundle.rename(root / "original-bundle")
            replacement = self.bundle(root)
            (replacement / "flow").write_text(
                "#!/bin/sh\n"
                f": > {shlex.quote(str(executed))}\n"
                "exit 0\n",
                encoding="utf-8",
            )
            (replacement / "flow").chmod(0o755)
            release.touch()
            _, stderr = process.communicate(timeout=5)

            self.assertNotEqual(process.returncode, 0, stderr)
            self.assertIn(b"installer bundle path changed during installation", stderr)
            self.assertFalse(executed.exists())
            self.assertEqual(list((prefix / "bin").iterdir()), [])

    def test_replacing_validated_target_directory_fails_without_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            validated = root / "validated"
            release = root / "release"
            self.pause_installer_after_validation(bundle, validated, release)
            prefix = root / "prefix"
            unrelated_cwd = root / "unrelated-cwd"
            unrelated_cwd.mkdir()
            process = subprocess.Popen(
                [
                    "/bin/sh",
                    str(bundle / "install.sh"),
                    "--prefix",
                    str(prefix),
                    "--no-default-executor",
                ],
                cwd=unrelated_cwd,
                env={"PATH": ""},
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )

            self.wait_for_installer_marker(process, validated)
            original_bin = root / "original-bin"
            (prefix / "bin").rename(original_bin)
            (prefix / "bin").mkdir(mode=0o755)
            release.touch()
            _, stderr = process.communicate(timeout=5)

            self.assertNotEqual(process.returncode, 0, stderr)
            self.assertIn(b"installation bin path changed during installation", stderr)
            self.assertEqual(list(original_bin.iterdir()), [])
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

    def test_signal_at_commit_boundary_is_all_or_nothing(self):
        commit = "installation_committed=1\n"
        cases = (
            ("before", f'/bin/kill -TERM "$$"\n{commit}', ()),
            (
                "after",
                f'{commit}/bin/kill -TERM "$$"\n',
                ("flow", "flow-executor"),
            ),
        )
        for side, injected, expected in cases:
            with self.subTest(side=side), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                bundle = self.bundle(root)
                installer = bundle / "install.sh"
                source = installer.read_text(encoding="utf-8")
                self.assertEqual(source.count(commit), 1)
                installer.write_text(
                    source.replace(commit, injected),
                    encoding="utf-8",
                )
                installer.chmod(0o755)
                prefix = root / "prefix"

                installed = self.install(bundle, prefix)

                self.assertEqual(installed.returncode, 128 + signal.SIGTERM, installed.stderr)
                self.assertEqual(
                    tuple(sorted(path.name for path in (prefix / "bin").iterdir())),
                    expected,
                )

    def test_signal_during_failure_cleanup_does_not_interrupt_rollback(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            bundle = self.bundle(root)
            installer = bundle / "install.sh"
            source = installer.read_text(encoding="utf-8")
            first_removal = '            /bin/rm -f -- "$executor_target" || :\n'
            self.assertEqual(source.count(first_removal), 1)
            installer.write_text(
                source.replace(
                    first_removal,
                    first_removal + '            /bin/kill -TERM "$$"\n',
                ),
                encoding="utf-8",
            )
            installer.chmod(0o755)
            (bundle / "flow").write_text("#!/bin/sh\nexit 65\n", encoding="utf-8")
            (bundle / "flow").chmod(0o755)
            prefix = root / "prefix"

            installed = self.install(bundle, prefix)

            self.assertEqual(installed.returncode, 1, installed.stderr)
            self.assertIn(b"failed readiness", installed.stderr)
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
