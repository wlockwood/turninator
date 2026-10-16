"""Generate reproducible dependency/font notices from locked, target-filtered Cargo metadata.

No downloads occur here. Missing crate-archive licenses require a checked,
commit-pinned entry in license-supplements.json; unknown omissions fail closed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def digest(data):
    return hashlib.sha256(data).hexdigest()


def resolved_packages(metadata):
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    pending, reached = [root], set()
    while pending:
        node = pending.pop()
        if node in reached:
            continue
        reached.add(node)
        pending.extend(dependency["pkg"] for dependency in nodes[node]["deps"])
    return sorted((package for package in metadata["packages"]
                   if package["id"] in reached and package["id"] != root),
                  key=lambda package: (package["name"], package["version"]))


def license_files(package):
    root = Path(package["manifest_path"]).parent.resolve()
    files = set()
    explicit = package.get("license_file")
    if explicit:
        files.add((root / explicit).resolve())
    for path in root.rglob("*"):
        name = path.name.lower()
        if path.is_file() and path.suffix.lower() not in (".rs", ".py", ".png", ".jpg") and (
            re.search(r"licen[cs]e|copying|copyright|notice", name)
            or name in ("ofl.txt", "ufl.txt", "hack-regular.txt")
        ):
            files.add(path.resolve())
    result = []
    for path in sorted(files):
        if not path.is_relative_to(root):
            raise ValueError(f"{package['name']}: license escapes crate directory")
        data = path.read_bytes()
        result.append({"path": path.relative_to(root).as_posix(), "sha256": digest(data),
                       "text": data.decode("utf-8-sig"), "url": None})
    return result


def render_notices(metadata, supplements):
    packages = resolved_packages(metadata)
    parts = ["# Third-party dependency and font notices\n",
             "Generated from Cargo.lock and the resolved Windows x64 dependency graph. "
             "Includes build dependencies as a conservative superset of the linked application. "
             "Versions and license expressions below are upstream package declarations; "
             "the reproduced texts retain their original attribution and terms.\n",
             "License hashes identify original source bytes; display normalizes line endings and trailing whitespace.\n",
             "This file does not assign a license to Turninator or the user's models. "
             "Blender, FFmpeg and ffprobe are not bundled. Their separate installations retain "
             "their own notices and terms. Embedded egui fonts include Hack, Noto Emoji, "
             "Ubuntu Light and emoji-icon-font; their cached font notices are included below.\n",
             f"Resolved third-party packages: **{len(packages)}**. "
             "Regenerate with tools/package.ps1 or tools/generate-notices.py; missing notices are an error.\n"]
    for package in packages:
        key = f"{package['name']}@{package['version']}"
        notices = license_files(package)
        for supplement in supplements.get(key, []):
            if digest(supplement["text"].encode("utf-8")) != supplement["sha256"]:
                raise ValueError(f"{key}: supplemental license hash mismatch")
            notices.append(supplement)
        # A font-specific license alone does not cover the Rust crate itself.
        if not notices or (package["name"] == "epaint_default_fonts" and key not in supplements):
            raise ValueError(f"{key}: no license text for package code; supply a pinned supplement")
        expression = package.get("license") or "See upstream license file"
        parts.append(f"\n## {package['name']} {package['version']}\n\nDeclared license: `{expression}`.\n")
        if package.get("repository"):
            parts.append(f"Upstream: {package['repository']}\n")
        for notice in notices:
            parts.append(f"\n### {notice['path']}\n\nSHA-256: `{notice['sha256']}`.\n")
            if notice.get("url"):
                parts.append(f"Source omitted from crate archive; retrieved at exact upstream commit: {notice['url']}\n")
            fence = "~" * max(4, max((len(match[0]) + 1 for match in re.finditer(r"~+", notice["text"])), default=4))
            parts.append(f"\n{fence}text\n{notice['text'].rstrip()}\n{fence}\n")
    return "\n".join(line.rstrip() for line in "\n".join(parts).splitlines()) + "\n"


def file_inventory(directory):
    root = Path(directory).resolve()
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"package must not contain symlink: {path.name}")
        if path.is_file():
            data = path.read_bytes()
            result[path.relative_to(root).as_posix()] = {"sha256": digest(data), "size_bytes": len(data)}
    return result


def render_native_notices(manifest, root):
    """Reproduce complete per-component text, including aggregate notices."""
    root = Path(root).resolve()
    parts = ["\n# Native renderer notices\n", f"Package variant: `{manifest['package_variant']}`.\n"]
    files = {entry["path"].lower(): entry for entry in manifest["files"]}
    for component in manifest["components"]:
        parts.append(f"\n## {component['id']} {component['version']}\n\nDeclared license: `{component['license']}`.\n")
        source = component.get("corresponding_source")
        if source:
            parts.append(f"\nCorresponding source: {source['url']}\n\nSHA-256: `{source['sha256']}`.\n\n{source['instructions']}\n")
        for reference in component["notices"]:
            # Match inventory's Windows case-insensitive reference lookup and
            # open/display the canonical inventory spelling on every host.
            entry = files[reference.lower()]
            name = entry["path"]
            path = (root / name).resolve()
            if not path.is_relative_to(root):
                raise ValueError("native notice path escapes package")
            data = path.read_bytes()
            if entry["role"] != "notice" or len(data) != entry["size_bytes"] or digest(data) != entry["sha256"]:
                raise ValueError(f"native notice identity mismatch: {name}")
            encoding = entry.get("text_encoding", "utf-8-sig")
            if encoding not in ("utf-8-sig", "latin-1"):
                raise ValueError(f"unsupported native notice encoding: {name}")
            text = data.decode(encoding)
            fence = "~" * max(4, max((len(match[0]) + 1 for match in re.finditer(r"~+", text)), default=4))
            parts.append(f"\n### {name}\n\nSHA-256: `{entry['sha256']}`.\n\n{fence}text\n{text.rstrip()}\n{fence}\n")
    return "\n".join(line.rstrip() for line in "\n".join(parts).splitlines()) + "\n"


def command_output(args, cwd):
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, check=True,
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    return result.stdout.strip()


def write_manifest(package_dir, source_root, metadata, profile, tools, prebuilt=False):
    package = Path(package_dir).resolve()
    root = Path(source_root).resolve()
    app = next(p for p in metadata["packages"] if p["id"] == metadata["resolve"]["root"])
    sources = {}
    for directory in ("src", "renderer", "tools", "native", "assets/render"):
        for path in sorted((root / directory).rglob("*")):
            if path.is_file() and path.suffix in (".rs", ".wgsl", ".py", ".ps1", ".json", ".cpp", ".h", ".hpp", ".mat", ".txt", ".patch", ".cmake"):
                sources[path.relative_to(root).as_posix()] = digest(path.read_bytes())
    for name in ("Cargo.toml", "Cargo.lock", "README.md", "docs/runtime-support.md", "THIRD-PARTY-NOTICES.md"):
        sources[name] = digest((root / name).read_bytes())
    if (root / "build.rs").is_file():
        sources["build.rs"] = digest((root / "build.rs").read_bytes())
    tool_identities = []
    for value in tools:
        path = Path(value).resolve()
        entry = {"name": path.name, "bundled": False, "available_on_packaging_machine": path.is_file()}
        if path.is_file():
            entry["sha256"] = digest(path.read_bytes())
            entry["version"] = command_output([str(path), "-version" if path.stem.lower().startswith("ff") else "--version"], root).splitlines()[0]
        tool_identities.append(entry)
    manifest = {"schema_version": 1, "application": app["name"], "version": app["version"],
                "target": "x86_64-pc-windows-msvc", "build_profile": profile,
                "built_by_packaging_command": not prebuilt,
                "source_revision": command_output(["git", "rev-parse", "HEAD"], root),
                "source_dirty": bool(command_output(["git", "status", "--porcelain", "--untracked-files=normal"], root)),
                "compiler": command_output(["rustc", "--version"], root),
                "source_sha256": sources, "files": file_inventory(package),
                "external_tools": tool_identities,
                "dependencies": [{"name": p["name"], "version": p["version"], "license": p["license"]}
                                 for p in resolved_packages(metadata)]}
    (package / "version-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--supplements", type=Path, default=Path(__file__).with_name("license-supplements.json"))
    parser.add_argument("--package", type=Path)
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--profile", default="release")
    parser.add_argument("--tool", action="append", default=[])
    parser.add_argument("--prebuilt", action="store_true")
    parser.add_argument("--native-manifest", type=Path)
    parser.add_argument("--native-root", type=Path)
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text(encoding="utf-8-sig"))
    supplements = json.loads(args.supplements.read_text(encoding="utf-8")) if args.supplements.exists() else {}
    text = render_notices(metadata, supplements)
    if bool(args.native_manifest) != bool(args.native_root):
        parser.error("--native-manifest and --native-root must be supplied together")
    if args.native_manifest:
        native = json.loads(args.native_manifest.read_text(encoding="utf-8-sig"))
        text += render_native_notices(native, args.native_root)
    args.output.write_text(text, encoding="utf-8", newline="\n")
    if args.package:
        write_manifest(args.package, args.source_root, metadata, args.profile, args.tool, args.prebuilt)
    print(f"Notices: {len(resolved_packages(metadata))} exact resolved third-party packages -> {args.output}")


if __name__ == "__main__":
    main()
