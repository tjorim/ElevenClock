"""Development-only release assembly and fail-closed validation (Python 3.11+)."""
import argparse
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"
SYSTEM_DLLS = {
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "comctl32.dll",
    "comdlg32.dll", "crypt32.dll", "d2d1.dll", "dwrite.dll", "dwmapi.dll",
    "dxgi.dll", "gdi32.dll", "imm32.dll", "iphlpapi.dll", "kernel32.dll",
    "kernelbase.dll", "msimg32.dll", "netapi32.dll", "ntdll.dll", "ole32.dll",
    "oleacc.dll", "oleaut32.dll", "powrprof.dll", "propsys.dll", "psapi.dll",
    "rpcrt4.dll", "secur32.dll", "setupapi.dll", "shell32.dll", "shlwapi.dll",
    "synchronization.dll", "uiautomationcore.dll", "user32.dll", "userenv.dll",
    "usp10.dll", "uxtheme.dll", "version.dll", "winhttp.dll", "winmm.dll",
    "winspool.drv", "ws2_32.dll", "wtsapi32.dll",
}


def run(*args):
    return subprocess.check_output(args, cwd=ROOT).decode("utf-8").strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def check_gates(gates):
    for name in ("licensing", "windows_acceptance", "clean_standard_account"):
        gate = gates.get(name, {})
        if gate.get("passed") is not True or not gate.get("evidence"):
            raise ValueError(f"Distribution blocked: {name}: {gate.get('evidence', 'missing evidence')}")


def inspect_pe(data):
    """Read the PE32+ import table without executing the candidate."""
    if data[:2] != b"MZ":
        raise ValueError("Not a PE executable")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Invalid PE signature")
    machine, count = struct.unpack_from("<HH", data, pe + 4)
    size = struct.unpack_from("<H", data, pe + 20)[0]
    opt = pe + 24
    if machine != 0x8664 or struct.unpack_from("<H", data, opt)[0] != 0x20B:
        raise ValueError("Expected native x64 PE32+")
    if struct.unpack_from("<H", data, opt + 68)[0] != 2:
        raise ValueError("Expected GUI subsystem (no console)")
    sections = []
    for i in range(count):
        sections.append(struct.unpack_from("<IIII", data, opt + size + i * 40 + 8))

    def offset(rva):
        for virtual_size, address, raw_size, raw in sections:
            if address <= rva < address + max(virtual_size, raw_size):
                delta = rva - address
                if delta >= raw_size or raw + delta >= len(data):
                    break
                return raw + delta
        raise ValueError(f"Unmapped PE RVA: {rva:#x}")

    # Fail closed rather than silently omitting a second dependency mechanism.
    if struct.unpack_from("<I", data, opt + 112 + 13 * 8)[0]:
        raise ValueError("Delay imports require explicit runtime review")
    imports = set()
    rva = struct.unpack_from("<I", data, opt + 120)[0]
    if not rva:
        raise ValueError("Missing native imports")
    pos = offset(rva)
    while True:
        descriptor = struct.unpack_from("<IIIII", data, pos)
        if not any(descriptor):
            break
        name = offset(descriptor[3])
        imports.add(data[name:data.index(b"\0", name)].decode("ascii").lower())
        pos += 20
    for name in sorted(imports):
        api_set = name.startswith(("api-ms-win-", "ext-ms-win-")) and name.endswith(".dll")
        if "crt" in name or name.startswith(("msvcp", "vcruntime")):
            raise ValueError(f"Dynamic CRT dependency is forbidden: {name}")
        if name not in SYSTEM_DLLS and not api_set:
            raise ValueError(f"Unreviewed native dependency: {name}")
    return sorted(imports)


def zip_tree(source, destination):
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as archive:
        for file in sorted(source.rglob("*")):
            if file.is_file():
                if file.is_symlink():
                    raise ValueError(f"Refusing source symlink: {file}")
                archive.write(file, file.relative_to(source).as_posix())


def prepare(out, mode):
    if out.exists():
        raise ValueError("Output must be a new directory; stale artifacts are not reusable")
    if out.is_relative_to(ROOT) and not out.is_relative_to(ROOT / "dist"):
        raise ValueError("In-repository output must be under ignored dist/ to avoid capturing itself")
    gates = json.loads((ROOT / "packaging/release-gates.json").read_text())
    dirty = bool(run("git", "status", "--porcelain"))
    if mode == "Distribution":
        check_gates(gates)
        if dirty:
            raise ValueError("Distribution requires a clean source checkout")
        # The candidate readme deliberately says development/blocked. A release
        # decision must update this wording, not merely flip a boolean.
        if "Distribution is blocked" in (ROOT / "packaging/PACKAGE-README.txt").read_text():
            raise ValueError("Distribution requires reviewed release notice wording")
    out.mkdir(parents=True)
    payload = out / "payload"
    payload.mkdir()
    symbols = out / "symbols"
    symbols.mkdir()
    source = out / "source"
    source.mkdir()
    version = tomllib.loads((ROOT / "rust/Cargo.toml").read_text())["workspace"]["package"]["version"]
    name = f"ElevenClock-rs-{version}-{run('git', 'rev-parse', '--short=12', 'HEAD')}-x64"
    if mode == "Development":
        name += "-dev"
    binary_dir = ROOT / "rust/target/packaging" / TARGET / "release"
    binary = binary_dir / "elevenclock-rs.exe"
    imports = inspect_pe(binary.read_bytes())
    shutil.copy2(binary, payload / binary.name)
    pdb = binary_dir / "elevenclock_rs.pdb"
    if not pdb.is_file():
        raise ValueError("Missing release PDB")
    shutil.copy2(pdb, symbols / pdb.name)
    shutil.copy2(ROOT / "LICENSE", payload / "LICENSE.txt")
    shutil.copy2(ROOT / "packaging/PACKAGE-README.txt", payload / "PACKAGE-README.txt")
    spec = importlib.util.spec_from_file_location("license_check", ROOT / "scripts/rust-license-check.py")
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    metadata = json.loads(run("cargo", "metadata", "--manifest-path", "rust/Cargo.toml", "--locked", "--offline", "--format-version", "1"))
    notices = io.StringIO()
    if checker.inventory(metadata, out=notices):
        raise ValueError("Dependency license inventory failed")
    (payload / "THIRD-PARTY-NOTICES.txt").write_text(notices.getvalue(), encoding="utf-8")
    rust_docs = Path(run("rustc", "--print", "sysroot")) / "share/doc/rust"
    shutil.copy2(rust_docs / "COPYRIGHT.html", payload / "RUST-COPYRIGHT.html")
    # Exact working sources, including uncommitted development changes, instead of
    # falsely labelling git archive HEAD as the source of a modified binary.
    files = run("git", "ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0")
    source_hashes = {}
    for relative in sorted(set(files)):
        if not relative:
            continue
        file = ROOT / relative
        if not file.exists():
            continue  # A tracked deletion is absent from this exact source snapshot.
        if not file.is_file() or file.is_symlink() or not file.resolve().is_relative_to(ROOT):
            raise ValueError(f"Unsupported source entry: {relative}")
        target = source / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(file, target)
        source_hashes[relative] = digest(file)
    # GPL corresponding-source companion: include locked dependency sources and
    # Cargo's exact offline replacement config, rather than only a GitHub URL.
    config = run("cargo", "vendor", "--manifest-path", "rust/Cargo.toml", "--locked", "--offline", "--versioned-dirs", str(source / "vendor"))
    # Cargo emits an absolute path; make the source archive relocatable.
    config = config.replace(str(source / "vendor").replace("\\", "\\\\"), "vendor")
    config = config.replace((source / "vendor").as_posix(), "vendor")
    (source / ".cargo").mkdir(exist_ok=True)
    (source / ".cargo/config.toml").write_text(config + "\n", encoding="utf-8")
    write_json(source / "source-files.json", source_hashes)
    write_json(payload / "runtime-dependencies.json", {
        "target": TARGET, "crt": "static", "direct_imports": imports,
        "clean_machine_verified": gates["clean_standard_account"]["passed"],
        "note": "Import inspection does not prove clean-machine execution or runtime-loaded system components.",
    })
    provenance = {
        "version": version, "commit": run("git", "rev-parse", "HEAD"),
        "modified": dirty, "mode": mode, "target": TARGET,
        "rustc": run("rustc", "-Vv"), "cargo": run("cargo", "-V"),
        "rustflags": "-C target-feature=+crt-static", "signing": "unsigned",
        "exe_sha256": digest(binary), "lock_sha256": digest(ROOT / "rust/Cargo.lock"),
        "source_manifest_sha256": digest(source / "source-files.json"), "gates": gates,
        "source_archive": name + "-source.zip",
    }
    for directory in (payload, symbols, source):
        write_json(directory / "provenance.json", provenance)
    zip_tree(source, out / (name + "-source.zip"))
    zip_tree(payload, out / (name + "-portable.zip"))
    write_json(out / "package.json", {"name": name, "version": version})


def finish(out):
    package = json.loads((out / "package.json").read_text())
    name = package["name"]
    required = [out / (name + suffix) for suffix in ("-source.zip", "-portable.zip", "-setup.exe")]
    required += [out / "payload/elevenclock-rs.exe"]
    for file in required:
        if not file.is_file():
            raise ValueError(f"Missing artifact: {file}")
    # The checksum file sits alongside the downloaded archives/installer.
    (out / "SHA256SUMS.txt").write_text("".join(
        f"{digest(file)}  {file.relative_to(out).as_posix()}\n" for file in required
    ), encoding="utf-8")
    symbols = out / "symbols"
    (symbols / "SHA256SUMS.txt").write_text("".join(
        f"{digest(file)}  {file.name}\n" for file in sorted(symbols.iterdir())
        if file.name != "SHA256SUMS.txt"
    ), encoding="utf-8")


def verify(out):
    package = json.loads((out / "package.json").read_text())
    name = package["name"]
    expected = {name + suffix for suffix in ("-source.zip", "-portable.zip", "-setup.exe")}
    expected.add("payload/elevenclock-rs.exe")
    hashes = {}
    for line in (out / "SHA256SUMS.txt").read_text().splitlines():
        sha, relative = line.split("  ", 1)
        if relative not in expected or relative in hashes:
            raise ValueError(f"Unexpected/duplicate checksum entry: {relative}")
        if digest(out / relative) != sha:
            raise ValueError(f"Artifact checksum mismatch: {relative}")
        hashes[relative] = sha
    if set(hashes) != expected:
        raise ValueError("Incomplete artifact checksums")
    with zipfile.ZipFile(out / (name + "-portable.zip")) as archive:
        allowed = {"elevenclock-rs.exe", "LICENSE.txt", "THIRD-PARTY-NOTICES.txt",
                   "RUST-COPYRIGHT.html", "provenance.json", "runtime-dependencies.json", "PACKAGE-README.txt"}
        if set(archive.namelist()) != allowed or len(archive.namelist()) != len(allowed):
            raise ValueError("Portable ZIP file allowlist differs")
        if hashlib.sha256(archive.read("elevenclock-rs.exe")).hexdigest() != hashes["payload/elevenclock-rs.exe"]:
            raise ValueError("Portable executable differs from checksummed executable")
        provenance = json.loads(archive.read("provenance.json"))
    with zipfile.ZipFile(out / (name + "-source.zip")) as archive:
        manifest = archive.read("source-files.json")
        if hashlib.sha256(manifest).hexdigest() != provenance["source_manifest_sha256"]:
            raise ValueError("Source manifest does not match binary provenance")
        for relative, sha in json.loads(manifest).items():
            if hashlib.sha256(archive.read(relative)).hexdigest() != sha:
                raise ValueError(f"Source file differs from manifest: {relative}")
        config = tomllib.loads(archive.read(".cargo/config.toml").decode())
        if config["source"]["vendored-sources"]["directory"] != "vendor":
            raise ValueError("Source vendor config is not relocatable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "finish", "verify", "check-gates"))
    parser.add_argument("--output", type=Path, default=ROOT / "dist/candidate")
    parser.add_argument("--mode", choices=("Development", "Distribution"), default="Development")
    args = parser.parse_args()
    if args.action == "check-gates":
        check_gates(json.loads((ROOT / "packaging/release-gates.json").read_text()))
    elif args.action == "prepare":
        prepare(args.output.resolve(), args.mode)
    elif args.action == "finish":
        finish(args.output.resolve())
    else:
        verify(args.output.resolve())


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, struct.error, subprocess.CalledProcessError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)
