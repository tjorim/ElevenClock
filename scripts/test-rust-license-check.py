"""Regression tests for the fail-closed development notice inventory."""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("license_check", Path(__file__).with_name("rust-license-check.py"))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class LicenseCheckTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.package_dir = self.root / "crate"
        self.package_dir.mkdir()
        self.notice_root = self.root / "notices"
        self.notice_root.mkdir()
        self.write_manifest([])
        self.package = {
            "name": "fixture", "version": "1.0.0", "id": "fixture-id",
            "license": "MIT", "license_file": None,
            "manifest_path": str(self.package_dir / "Cargo.toml"),
        }

    def write_manifest(self, packages):
        (self.notice_root / "manifest.json").write_text(json.dumps({"packages": packages}))

    def run_inventory(self, workspace=False):
        out, err = io.StringIO(), io.StringIO()
        status = checker.inventory({
            "workspace_members": ["fixture-id"] if workspace else [],
            "packages": [self.package],
        }, self.notice_root, out, err)
        return status, out.getvalue(), err.getvalue()

    def test_reviewed_aliases_and_alternatives(self):
        for expression in [*checker.ALIASES, *checker.ALLOWED]:
            with self.subTest(expression=expression):
                self.assertTrue(checker.license_is_allowed(expression))

    def test_unreviewed_or_malformed_terms_still_fail(self):
        for expression in [None, "", "GPL-3.0-only", "MIT AND GPL-3.0-only",
                           "MIT OR Proprietary", "MIT/Proprietary", "MIT OR",
                           "Apache-2.0 WITH Unknown-exception", "BSD-2-Clause"]:
            with self.subTest(expression=expression):
                self.assertFalse(checker.license_is_allowed(expression))
        (self.package_dir / "LICENSE").write_text("original terms")
        self.package["license"] = "MIT OR Proprietary"
        self.assertEqual(self.run_inventory()[0], 1)

    def test_notices_are_retained_verbatim(self):
        text = "Copyright (c) Fixture\nOriginal permission notice.\n"
        (self.package_dir / "LICENSE-MIT").write_text(text)
        status, out, err = self.run_inventory()
        self.assertEqual((status, err), (0, ""))
        self.assertIn(text, out)

    def test_generic_authors_or_readme_are_not_notice_exemptions(self):
        (self.package_dir / "AUTHORS").write_text("Some names")
        (self.package_dir / "README.md").write_text("License: MIT")
        status, _, err = self.run_inventory()
        self.assertEqual(status, 1)
        self.assertIn("no license/notice files found", err)

    def add_mapping(self, bundled=False):
        base = self.package_dir if bundled else self.notice_root
        text = "Copyright Fixture\nOriginal MIT terms\n"
        path = base / "AUTHORS"
        path.write_text(text)
        self.write_manifest([{
            "name": "fixture", "version": "1.0.0", "license": "MIT",
            "bundled": [{"path": "AUTHORS", "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "source": "https://example.org/pinned/AUTHORS"}] if bundled else [],
            "supplemental": [] if bundled else [{"path": "AUTHORS", "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "source": "https://example.org/pinned/AUTHORS"}],
        }])
        return path, text

    def test_mapped_bundled_and_supplemental_notices(self):
        for bundled in [False, True]:
            with self.subTest(bundled=bundled):
                _, text = self.add_mapping(bundled)
                status, out, err = self.run_inventory()
                self.assertEqual((status, err), (0, ""))
                self.assertIn(text, out)
                self.assertIn("https://example.org/pinned/AUTHORS", out)

    def test_mapping_is_version_and_license_specific(self):
        self.add_mapping()
        self.package["version"] = "1.0.1"
        self.assertEqual(self.run_inventory()[0], 1)
        self.package["version"] = "1.0.0"
        self.package["license"] = "Apache-2.0"
        self.assertIn("license differs", self.run_inventory()[2])

    def test_corrupt_missing_or_empty_notices_fail(self):
        path, _ = self.add_mapping()
        path.write_text("modified")
        self.assertIn("digest mismatch", self.run_inventory()[2])
        path.unlink()
        self.assertIn("cannot read mapped notice", self.run_inventory()[2])
        (self.package_dir / "LICENSE").write_text(" \n")
        self.assertIn("empty notice", self.run_inventory()[2])

    def test_workspace_license_file_must_exist_and_be_readable(self):
        self.package["license"] = None
        self.package["license_file"] = "../LICENSE"
        self.assertEqual(self.run_inventory(workspace=True)[0], 1)
        (self.root / "LICENSE").write_text("Repository terms")
        self.assertEqual(self.run_inventory(workspace=True)[0], 0)

    def test_invalid_inputs_return_nonzero_without_traceback(self):
        original = checker.sys.argv
        self.addCleanup(setattr, checker.sys, "argv", original)
        checker.sys.argv = ["checker", str(self.root / "absent.json")]
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(checker.main(), 1)
        self.assertIn("cannot inventory", err.getvalue())


if __name__ == "__main__":
    unittest.main()
