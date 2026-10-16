"""Regression tests for binary and distribution gates, without running an app."""
import importlib.util
import json
from pathlib import Path
import struct
import os
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("rust-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def pe(dll=b"KERNEL32.dll", machine=0x8664, subsystem=2, delayed=False):
    data = bytearray(2048)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HH", data, 0x84, machine, 1)
    struct.pack_into("<H", data, 0x94, 240)
    opt = 0x98
    struct.pack_into("<H", data, opt, 0x20B)
    struct.pack_into("<H", data, opt + 68, subsystem)
    struct.pack_into("<II", data, opt + 120, 0x1000, 40)
    if delayed:
        struct.pack_into("<I", data, opt + 112 + 13 * 8, 0x1100)
    struct.pack_into("<IIII", data, opt + 240 + 8, 1024, 0x1000, 1024, 512)
    struct.pack_into("<IIIII", data, 512, 0, 0, 0, 0x1100, 0)
    data[768:768 + len(dll) + 1] = dll + b"\0"
    return data


class Gates(unittest.TestCase):
    def test_system_imports(self):
        self.assertEqual(release.inspect_pe(pe()), ["kernel32.dll"])
        self.assertEqual(release.inspect_pe(pe(b"api-ms-win-core-synch-l1-2-0.dll")), ["api-ms-win-core-synch-l1-2-0.dll"])

    def test_rejects_runtime_or_unreviewed_dll(self):
        for dll in (b"VCRUNTIME140.dll", b"ucrtbase.dll", b"api-ms-win-crt-runtime-l1-1-0.dll", b"Qt6Core.dll", b"python313.dll"):
            with self.subTest(dll=dll), self.assertRaises(ValueError):
                release.inspect_pe(pe(dll))

    def test_rejects_wrong_arch_console_and_unreviewed_delay_table(self):
        for args in ({"machine": 0xAA64}, {"subsystem": 3}, {"delayed": True}):
            with self.subTest(args=args), self.assertRaises(ValueError):
                release.inspect_pe(pe(**args))

    def test_rejects_invalid_pe(self):
        with self.assertRaises(ValueError):
            release.inspect_pe(b"not an executable")

    def test_each_release_gate_requires_a_real_boolean_and_evidence(self):
        keys = ("licensing", "windows_acceptance", "clean_standard_account")
        good = {key: {"passed": True, "evidence": "reviewed record"} for key in keys}
        release.check_gates(good)
        for key in keys:
            for bad in ({}, {"passed": "true", "evidence": "x"}, {"passed": True}, {"passed": False, "evidence": "pending"}):
                with self.subTest(key=key, bad=bad), self.assertRaises(ValueError):
                    release.check_gates({**good, key: bad})

    def test_current_distribution_is_blocked(self):
        gates = json.loads((release.ROOT / "packaging/release-gates.json").read_text())
        with self.assertRaisesRegex(ValueError, "Distribution blocked"):
            release.check_gates(gates)

    def test_vendor_source_timestamps_before_1980_can_be_archived(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            file = source / "LICENSE"
            file.write_bytes(b"original notice\n")
            os.utime(file, (1, 1))
            release.zip_tree(source, root / "source.zip")
            with zipfile.ZipFile(root / "source.zip") as archive:
                self.assertEqual(archive.read("LICENSE"), b"original notice\n")

    def test_finish_rejects_incomplete_artifact_set(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release.write_json(root / "package.json", {"name": "candidate"})
            with self.assertRaisesRegex(ValueError, "Missing artifact"):
                release.finish(root)


if __name__ == "__main__":
    unittest.main()
