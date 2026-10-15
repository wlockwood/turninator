"""Offline native payload validation. A manifest is a reviewed build assertion, not a signature.

The input directory is a closed payload: every file must be listed. Build logs,
link maps and source archives belong in separate qualification evidence. Never
infer redistribution permission or dynamic dependencies from PE imports alone.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import struct


def sha256(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def relative_path(value):
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        raise ValueError(f"unsafe relative path: {value!r}")
    path = PurePosixPath(value)
    if path.is_absolute() or path.as_posix() != value:
        raise ValueError(f"unsafe relative path: {value!r}")
    for part in path.parts:
        if part in (".", "..") or part.rstrip(" .") != part or re.search(r'[<>"|?*\x00-\x1f]', part):
            raise ValueError(f"unsafe relative path: {value!r}")
        if re.fullmatch(r"(?i)(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?", part):
            raise ValueError(f"reserved Windows path: {value!r}")
    return path


def no_reparse(path):
    path = Path(path).absolute()
    for part in (path, *path.parents):
        info = part.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError(f"reparse point is not a package input: {part.name}")


def pe_imports(data):
    """Read x64 PE32+ normal/delay import names without loading executable code."""
    def unpack(fmt, offset):
        if offset < 0 or offset + struct.calcsize(fmt) > len(data):
            raise ValueError("truncated PE image")
        return struct.unpack_from(fmt, data, offset)

    if data[:2] != b"MZ":
        raise ValueError("native payload is not PE")
    header = unpack("<I", 60)[0]
    if data[header:header + 4] != b"PE\0\0":
        raise ValueError("invalid PE signature")
    machine, count = unpack("<HH", header + 4)
    optional = header + 24
    optional_size = unpack("<H", header + 20)[0]
    if machine != 0x8664 or unpack("<H", optional)[0] != 0x20b or optional_size < 224:
        raise ValueError("native payload requires x64 PE32+")
    directories = unpack("<I", optional + 108)[0]
    if directories < 14 or optional_size < 112 + 14 * 8:
        raise ValueError("missing PE directory table")
    sections = []
    for index in range(count):
        virtual_size, address, size, offset = unpack("<IIII", optional + optional_size + index * 40 + 8)
        if offset + size > len(data):
            raise ValueError("truncated PE section")
        sections.append((address, size, offset))

    def file_offset(rva, length):
        for address, size, offset in sections:
            if address <= rva and rva + length <= address + size:
                return offset + rva - address
        raise ValueError("PE RVA outside file-backed sections")

    names = set()
    for directory, stride in ((1, 20), (13, 32)):
        rva, size = unpack("<II", optional + 112 + directory * 8)
        if not rva and not size:
            continue
        terminated = False
        for offset in range(0, size - stride + 1, stride):
            record = file_offset(rva + offset, stride)
            fields = unpack("<" + "I" * (stride // 4), record)
            if not any(fields):
                terminated = True
                break
            if directory == 13 and fields[0] != 1:
                raise ValueError("unsupported delay-import address format")
            name_rva = fields[3] if directory == 1 else fields[1]
            raw = bytearray()
            for index in range(260):
                byte = data[file_offset(name_rva + index, 1)]
                if not byte:
                    break
                raw.append(byte)
            else:
                raise ValueError("unterminated PE import name")
            try:
                name = raw.decode("ascii").lower()
            except UnicodeDecodeError as error:
                raise ValueError("invalid PE import name") from error
            if not re.fullmatch(r"[a-z0-9_.-]+\.dll", name):
                raise ValueError("invalid PE import name")
            names.add(name)
        if not terminated:
            raise ValueError("unterminated PE import directory")
    return names


# Windows 10/11 system components only. MSVC/OpenMP are deliberately absent.
SYSTEM_DLLS = frozenset("kernel32.dll user32.dll gdi32.dll advapi32.dll shell32.dll ole32.dll oleaut32.dll "
    "comdlg32.dll comctl32.dll shlwapi.dll version.dll ws2_32.dll winmm.dll bcrypt.dll ncrypt.dll "
    "crypt32.dll secur32.dll setupapi.dll cfgmgr32.dll rpcrt4.dll powrprof.dll iphlpapi.dll "
    "ntdll.dll psapi.dll dbghelp.dll dxgi.dll d3d11.dll d3d12.dll opengl32.dll glu32.dll "
    "dwmapi.dll imm32.dll uxtheme.dll normaliz.dll usp10.dll wintrust.dll winhttp.dll "
    "wtsapi32.dll userenv.dll msvcrt.dll ucrtbase.dll".split())


def portable_metadata(value):
    if isinstance(value, dict):
        for key, item in value.items():
            portable_metadata(key)
            portable_metadata(item)
    elif isinstance(value, list):
        for item in value:
            portable_metadata(item)
    elif isinstance(value, str) and (value.startswith("\\") or (value.startswith("/") and value not in ("/MD", "/MT")) or re.search(r"(?:^|\s)[A-Za-z]:[/\\]", value)):
        raise ValueError("absolute build-machine paths are forbidden in product metadata")


def validate_inventory(manifest, runtime, lock_path):
    runtime = Path(runtime).absolute()
    no_reparse(runtime)
    portable_metadata(manifest)
    if manifest.get("schema_version") != 1 or manifest.get("sdk_lock_sha256") != sha256(lock_path):
        raise ValueError("native SDK lock identity mismatch")
    components = {}
    for component in manifest["components"]:
        name = component["id"]
        if not re.fullmatch(r"[a-z0-9_.-]+", name) or name in components:
            raise ValueError("invalid or duplicate component")
        if not component.get("version") or not component.get("license") or not component.get("notices"):
            raise ValueError(f"{name}: version, license and full notices required")
        if "MPL" in component["license"]:
            source = component.get("corresponding_source", {})
            if not re.fullmatch(r"[a-f0-9]{64}", source.get("sha256", "")) or not source.get("url", "").startswith("https://") or not source.get("instructions"):
                raise ValueError(f"{name}: exact corresponding source availability required")
        components[name] = component
    entries = {}
    for entry in manifest["files"]:
        name = relative_path(entry["path"]).as_posix()
        if name.lower() in entries:
            raise ValueError(f"duplicate Windows destination: {name}")
        if entry["component"] not in components or entry["role"] not in ("bridge", "runtime", "asset", "notice", "source", "receipt"):
            raise ValueError(f"invalid component or role: {name}")
        namespace = "notices/" if entry["role"] == "notice" else "sources/" if entry["role"] == "source" else "renderers/"
        if not name.startswith(namespace):
            raise ValueError(f"native file outside its package namespace: {name}")
        path = runtime / name
        no_reparse(path)
        if not path.is_file() or path.stat().st_size != entry["size_bytes"] or sha256(path) != entry["sha256"]:
            raise ValueError(f"native file identity mismatch: {name}")
        entries[name.lower()] = entry
    actual = set()
    for path in runtime.rglob("*"):
        no_reparse(path)
        if path.is_file():
            actual.add(path.relative_to(runtime).as_posix().lower())
    if actual != set(entries):
        raise ValueError(f"unlisted or missing native payload: {sorted(actual ^ set(entries))}")
    for component in components.values():
        for name in component["notices"]:
            relative_path(name)
            entry = entries.get(name.lower())
            if not entry or entry["role"] != "notice" or entry["component"] != component["id"]:
                raise ValueError(f"{component['id']}: missing full component notice: {name}")
    for entry in entries.values():
        path = PurePosixPath(entry["path"])
        if path.suffix.lower() not in (".dll", ".exe"):
            continue
        if path.suffix.lower() == ".exe":
            raise ValueError("native renderer bundle must not ship helper executables")
        for dependency in pe_imports((runtime / path).read_bytes()):
            if dependency in SYSTEM_DLLS or dependency.startswith(("api-ms-win-", "ext-ms-win-")):
                continue
            key = (path.parent / dependency).as_posix().lower()
            if key not in entries:
                raise ValueError(f"{path}: missing local dependency {dependency}")
    return manifest


def validate_full(manifest, runtime, lock_path):
    validate_inventory(manifest, runtime, lock_path)
    if set(manifest["backends"]) != {"filament", "luxcore"}:
        raise ValueError("full packaging requires both filament and luxcore")
    # LuxCore receipt/attestation must come from the reviewed Task8b contract.
    # Until integrated, refusing full mode is intentional; no invented receipt.
    raise ValueError("full packaging awaits reviewed LuxCore receipt contract and qualification closure")


def filament_receipt(bridge, lock_path, attestation):
    """Bind a reviewed build attestation to the exact bytes accepted by Task7b.

    This does not turn an unreviewed document into qualified build provenance.
    Review must establish source/input/asset completeness and qualification;
    this final step rejects stale bridge/lock/import/compiler assertions.
    """
    lock = json.loads(Path(lock_path).read_text(encoding="utf-8-sig"))
    if attestation.get("schema_version") != 1 or attestation.get("backend") != "filament":
        raise ValueError("Filament requires a schema-1 reviewed build attestation")
    if attestation.get("sdk_lock_sha256") != sha256(lock_path):
        raise ValueError("build attestation SDK lock mismatch")
    if attestation.get("bridge_sha256") != sha256(bridge):
        raise ValueError("build attestation bridge identity mismatch")
    if attestation.get("compiler") != lock["toolchain"]["compiler"]:
        raise ValueError("build attestation compiler mismatch")
    if not re.fullmatch("[a-f0-9]{40}", attestation.get("source_revision", "")):
        raise ValueError("build attestation requires a source revision")
    for key in ("source_files", "static_inputs", "assets", "build_commands", "tool_versions"):
        if not attestation.get(key):
            raise ValueError(f"build attestation missing {key}")
    imports = sorted(pe_imports(Path(bridge).read_bytes()))
    if attestation.get("imports") != imports:
        raise ValueError("build attestation imports mismatch")
    version = lock["filament"]["version"] + "+" + lock["filament"]["commit"]
    archives = [entry for entry in lock["archives"] if entry["backend"] == "filament"]
    if len(archives) != 1:
        raise ValueError("ambiguous Filament SDK archive")
    return {"schema_version": 1, "sdk_version": version, "sdk_archive_sha256": archives[0]["sha256"],
            "native_recipe": "turninator-filament-native-v1", "bridge_sha256": attestation["bridge_sha256"]}


def core_manifest():
    return {"schema_version": 1, "package_variant": "core", "files": [], "components": [],
            "backends": {name: {"status": "not_bundled", "reason": "Optional native SDK omitted from core development package"}
                         for name in ("filament", "luxcore")}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", choices=("full", "core"), required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--sdk-lock", type=Path, default=Path(__file__).resolve().parents[1] / "native/sdk-lock.json")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.variant == "core":
        if args.manifest or args.runtime:
            parser.error("core mode must not accept native SDK inputs")
        manifest = core_manifest()
    else:
        if not args.manifest or not args.runtime:
            parser.error("full mode requires --manifest and --runtime")
        manifest = json.loads(args.manifest.read_text(encoding="utf-8-sig"))
        validate_full(manifest, args.runtime, args.sdk_lock)
    if args.output:
        args.output.mkdir(parents=True, exist_ok=True)
        no_reparse(args.output)
        for entry in manifest["files"]:
            destination = args.output / entry["path"]
            if destination.exists():
                raise ValueError(f"existing package destination: {entry['path']}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            no_reparse(destination.parent)
            shutil.copyfile(args.runtime / entry["path"], destination)
        target = args.output / "native-sdk-manifest.json"
        with target.open("x", encoding="utf-8") as output:
            json.dump(manifest, output, indent=2)
            output.write("\n")
    print(f"Validated {args.variant} native inventory")


if __name__ == "__main__":
    main()
