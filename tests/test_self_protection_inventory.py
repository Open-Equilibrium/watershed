"""Synthetic admission behavior for the dev-only native protection experiment."""

import importlib.util
import os
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import SimpleNamespace


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).resolve().parents[1] / "scripts" / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CASES = load("native_cases", "macos-self-protection-cases.py")
PROBE = load("native_probe", "probe-macos-self-protection.py")


class ProtectedInventory(unittest.TestCase):
    def test_internal_publication_alias_is_admitted_but_external_alias_is_not(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            home, store = root / "home", root / "store"
            home.mkdir(); store.mkdir()
            staged = home / "stage"; staged.write_bytes(b"fixture")
            published = home / "published"; os.link(staged, published)
            self.assertEqual(CASES.admit([home, store], []), 2)
            os.link(staged, root / "outside")
            with self.assertRaisesRegex(ValueError, "external hardlink"):
                CASES.admit([home, store], [])

    def test_selected_image_and_separate_homes_are_in_the_same_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            homes = [root / "first", root / "second"]
            for home in homes: home.mkdir()
            image = root / "image"; image.write_bytes(b"fixture")
            os.link(image, homes[1] / "internal-alias")
            self.assertEqual(CASES.admit(homes, [image]), 2)
            with self.assertRaisesRegex(ValueError, "external hardlink"):
                CASES.admit(homes[:1], [image])
            with self.assertRaises(FileNotFoundError):
                CASES.admit(homes, [root / "missing-image"])

    def test_overlapping_roots_do_not_double_count_links_or_exceed_scan_bound(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            nested = root / "nested"; nested.mkdir()
            record = nested / "record"; record.write_bytes(b"fixture")
            self.assertEqual(CASES.admit([root, nested], [record]), 1)
            with self.assertRaisesRegex(ValueError, "inventory limit"):
                CASES.admit([root], [], limit=0)


class DetachedHelperResult(unittest.TestCase):
    def test_result_is_read_only_after_the_helper_finishes_writing_it(self):
        helpers = []

        def invoke(command, cwd, log):
            # Stands in for the detached helper: the leader exits at once, then the
            # helper writes and publishes its result with a pause after creating it.
            target, release, result = map(Path, command[-3:])

            def helper():
                while not release.exists(): time.sleep(.01)
                target.write_bytes(PROBE.CHANGED)
                with result.open("w") as stream:
                    time.sleep(.2)
                    stream.write('{"state": "ok"}')

            helpers.append(threading.Thread(target=helper)); helpers[0].start()
            return {"launched": True, "returncode": 0, "timeout": False}

        probe = SimpleNamespace(fixture=PROBE.fixture, invoke=invoke,
                                read_marker=PROBE.read_marker, unchanged=PROBE.unchanged)
        with tempfile.TemporaryDirectory() as temporary:
            row = CASES.detached(probe, Path(temporary), False)
            helpers[0].join()
        self.assertEqual((row["outcome"], row["child"]), ("pass", {"state": "ok"}))


if __name__ == "__main__": unittest.main()
