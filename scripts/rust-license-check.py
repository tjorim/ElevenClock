"""CI development tool: inventory licenses, reject unreviewed dependency terms.

This is not shipped in the app. It retains original license/NOTICE text instead
of treating a package's SPDX metadata as a replacement for attribution.
"""
import json
from pathlib import Path
import sys

ALLOWED = {
    "MIT", "Apache-2.0", "MIT OR Apache-2.0", "Apache-2.0 OR MIT",
    "BSD-3-Clause", "Unicode-3.0", "(MIT OR Apache-2.0) AND Unicode-3.0", "Unlicense OR MIT",
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
}
metadata = json.loads(Path(sys.argv[1]).read_text())
workspace = set(metadata["workspace_members"])
failed = False
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    license_id = package["license"]
    print(f"\n{package['name']} {package['version']}: {license_id or 'repository license-file'}")
    if package["id"] not in workspace and license_id not in ALLOWED:
        print(f"ERROR: {package['name']} {package['version']} license requires review: {license_id}", file=sys.stderr)
        failed = True
    directory = Path(package["manifest_path"]).parent
    notices = sorted(p for p in directory.iterdir() if p.is_file() and
                     p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")))
    if package["license_file"]:
        explicit = directory / package["license_file"]
        if explicit not in notices:
            notices.append(explicit)
    if not notices:
        print("ERROR: no license/notice files found", file=sys.stderr)
        failed = True
    for notice in notices:
        print(f"--- {notice.name} ---")
        print(notice.read_text(errors="replace"))
if failed:
    sys.exit(1)
