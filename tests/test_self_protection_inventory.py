"""Synthetic admission behavior for the dev-only native protection experiment."""

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "native_cases", Path(__file__).resolve().parents[1] / "scripts" / "macos-self-protection-cases.py"
)
CASES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CASES)


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


if __name__ == "__main__": unittest.main()
