#!/usr/bin/env python3
"""Builds plugins for wasm32-wasip2 and packs each as dist/<name>-<version>.noriplugin (its plugin.wasm, its
strings/, and its icon.png and README.md if it has them); the version is the one the SDK's plugin! wrote into the wasm.

    tools/pack.py [name ...]             every plugin, or those named
    tools/pack.py name --release         and adds that version to index.json, the repo nori reads, at
                                         the URL of the release tagged <name>-<version>
    tools/pack.py --links                points index.json at each plugin's README and icon, builds nothing

A release is made by the release workflow when a tag <name>-<version> is pushed; index.json is never
written by hand."""

import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tomllib
import zipfile

root = pathlib.Path(__file__).resolve().parent.parent
dist = root / "dist"


def links(name):
    """Plugin `name`'s README and icon on the main branch: what a repo's page shows of every version."""
    raw = f"https://raw.githubusercontent.com/norifm/nori-plugins/main/{name}"
    return {key: f"{raw}/{file}" for key, file in (("readme", "README.md"), ("icon", "icon.png")) if (root / name / file).exists()}


def leb(b, at):
    """An unsigned LEB128 at `at`: its value and where it ends."""
    v, shift = 0, 0
    while True:
        byte = b[at]
        at += 1
        v |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return v, at


def section(wasm, name):
    """The first custom section `name` in a module or component, nested modules and components included."""
    at = 8
    while at < len(wasm):
        kind, size_at = wasm[at], at + 1
        size, start = leb(wasm, size_at)
        body = wasm[start:start + size]
        if kind == 0:
            n, s = leb(body, 0)
            if body[s:s + n].decode() == name:
                return body[s + n:]
        # A component's nested core module (1) or component (4).
        elif wasm[4:8] == b"\x0d\x00\x01\x00" and kind in (1, 4):
            found = section(body, name)
            if found is not None:
                return found
        at = start + size
    return None


def manifest(wasm):
    """The id, version and API version the SDK's plugin! wrote."""
    b = section(wasm, "nori:plugin")
    if b is None:
        sys.exit("no nori:plugin section: built without the SDK's plugin!")
    at = 1
    def text():
        nonlocal at
        n = int.from_bytes(b[at:at + 2], "little")
        at += 2 + n
        return b[at - n:at].decode()
    plugin_id, version = text(), text()
    return plugin_id, version, int.from_bytes(b[at:at + 4], "little")



REPO = "https://github.com/norifm/nori-plugins"

args = [a for a in sys.argv[1:] if not a.startswith("--")]
release = "--release" in sys.argv
names = args or [p.parent.name for p in sorted(root.glob("*/Cargo.toml"))]
if release and len(names) != 1:
    sys.exit("--release packs one plugin")
dist.mkdir(exist_ok=True)
# Panic messages keep source paths: named the same on every machine, a tag builds to the same bytes anywhere.
cargo_home = os.environ.get("CARGO_HOME", pathlib.Path.home() / ".cargo")
env = {**os.environ, "RUSTFLAGS": f"--remap-path-prefix={root}=nori-plugins --remap-path-prefix={cargo_home}=cargo"}
if "--links" in sys.argv:
    index_file = root / "index.json"
    index = json.loads(index_file.read_text())
    for entry in index["plugins"]:
        entry.update(links(entry["source"].rsplit("/", 1)[1]))
    index_file.write_text(json.dumps(index, indent=2) + "\n")
    sys.exit()

for name in names:
    # One plugin per build, as its release builds it: several at once share their dependencies' features.
    subprocess.run(["cargo", "build", "-j4", "--release", "--target", "wasm32-wasip2", "-p", name], cwd=root, check=True, env=env)
    wasm = (root / "target/wasm32-wasip2/release" / f"{name.replace('-', '_')}.wasm").read_bytes()
    plugin_id, version, api = manifest(wasm)
    out = dist / f"{name}-{version}.noriplugin"
    files = [("plugin.wasm", wasm)] + [(f"strings/{s.name}", s.read_bytes()) for s in sorted((root / name / "strings").glob("*.toml"))]
    files += [(extra, (root / name / extra).read_bytes()) for extra in ("icon.png", "README.md") if (root / name / extra).exists()]
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for path, data in files:
            # A fixed date: the same files pack to the same bytes, and the same sha256.
            z.writestr(zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0)), data, zipfile.ZIP_DEFLATED)
    print(out.relative_to(root))
    if not release:
        continue
    tag = f"{name}-{version}"
    index_file = root / "index.json"
    index = json.loads(index_file.read_text()) if index_file.exists() else {"plugins": []}
    words = tomllib.loads((root / name / "strings/en.toml").read_text())
    entry = next((p for p in index["plugins"] if p["id"] == plugin_id), None)
    if entry is None:
        entry = {"id": plugin_id, "versions": []}
        index["plugins"].append(entry)
    # The newest release names the plugin; every version stays listed.
    entry.update({"name": words.get("name", plugin_id), "about": words.get("about"), "source": f"{REPO}/tree/{tag}/{name}", **links(name)})
    entry["versions"] = [v for v in entry["versions"] if v["version"] != version]
    entry["versions"].insert(0, {
        "version": version,
        "api": api,
        "released": datetime.date.today().isoformat(),
        "url": f"{REPO}/releases/download/{tag}/{out.name}",
        "sha256": hashlib.sha256(out.read_bytes()).hexdigest(),
    })
    index["plugins"].sort(key=lambda p: p["id"])
    index_file.write_text(json.dumps(index, indent=2) + "\n")
    print(index_file.relative_to(root))
