"""Inventory original notices; fail on unreviewed terms or missing notice text.

Development only. Supplemental notices are offline, version-specific copies of
upstream originals, verified by digest. SPDX metadata never replaces attribution.
"""
import hashlib
import json
from pathlib import Path
import sys

ALLOWED = {
    "MIT", "Apache-2.0", "MIT OR Apache-2.0", "Apache-2.0 OR MIT",
    "BSD-3-Clause", "Unicode-3.0", "(MIT OR Apache-2.0) AND Unicode-3.0", "Unlicense OR MIT",
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
    # Reviewed OR alternatives: select MIT, not the other offered terms.
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    "BSD-2-Clause OR Apache-2.0 OR MIT",
}
# Historical Cargo dual-license notation. Do not rewrite arbitrary slash terms.
ALIASES = {
    "MIT/Apache-2.0": "MIT OR Apache-2.0",
    "Apache-2.0 / MIT": "Apache-2.0 OR MIT",
    "Unlicense/MIT": "Unlicense OR MIT",
}
NOTICE_ROOT = Path(__file__).resolve().parent.parent / "docs/dependency-notices"


def license_is_allowed(expression):
    return ALIASES.get(expression, expression) in ALLOWED


def inventory(metadata, notice_root=NOTICE_ROOT, out=None, err=None):
    out = sys.stdout if out is None else out
    err = sys.stderr if err is None else err
    workspace = set(metadata["workspace_members"])
    manifest = json.loads((notice_root / "manifest.json").read_text())
    supplements = {(p["name"], p["version"]): p for p in manifest["packages"]}
    failed = False

    def error(message):
        nonlocal failed
        print(f"ERROR: {message}", file=err)
        failed = True

    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        label = f"{package['name']} {package['version']}"
        license_id = package["license"]
        print(f"\n{label}: {license_id or 'repository license-file'}", file=out)
        if package["id"] not in workspace and not license_is_allowed(license_id):
            error(f"{label} license requires review: {license_id}")
        directory = Path(package["manifest_path"]).parent
        notices = sorted(p for p in directory.iterdir() if p.is_file() and
                         p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")))
        if package["license_file"]:
            explicit = directory / package["license_file"]
            if explicit not in notices:
                notices.append(explicit)
        supplement = supplements.get((package["name"], package["version"]))
        if supplement:
            if supplement["license"] != license_id:
                error(f"{label} notice mapping license differs from metadata")
            else:
                for entry, base in (
                    [(entry, directory) for entry in supplement["bundled"]]
                    + [(entry, notice_root) for entry in supplement["supplemental"]]
                ):
                    path = base / entry["path"]
                    try:
                        data = path.read_bytes()
                    except OSError as exc:
                        error(f"{label} cannot read mapped notice {path}: {exc}")
                        continue
                    if hashlib.sha256(data).hexdigest() != entry["sha256"]:
                        error(f"{label} mapped notice digest mismatch: {path}")
                        continue
                    print(f"--- upstream original: {entry['source']} ---", file=out)
                    if path not in notices:
                        notices.append(path)
        if not notices:
            error(f"{label}: no license/notice files found")
        for notice in notices:
            try:
                text = notice.read_text(encoding="utf-8")
                if not text.strip():
                    error(f"{label}: empty notice {notice}")
                    continue
            except (OSError, UnicodeError) as exc:
                error(f"{label} cannot read notice {notice}: {exc}")
                continue
            print(f"--- {notice.name} ---", file=out)
            print(text, file=out)
    return 1 if failed else 0


def main():
    try:
        return inventory(json.loads(Path(sys.argv[1]).read_text()))
    except (OSError, ValueError, KeyError, IndexError, TypeError) as exc:
        print(f"ERROR: cannot inventory dependency notices: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
