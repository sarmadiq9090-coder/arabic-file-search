#!/usr/bin/env python3
"""Lay out one MSIX package folder per CPU for makeappx (store.yml).

    python3 packaging/msix/make_layout.py --exes DIR --out OUT [--version 0.9.0]

DIR holds the release exes: bahth.exe (x64), bahth-x86.exe, bahth-arm64.exe.
For each one present, OUT/<arch>/ gets bahth.exe, Assets/ and an
AppxManifest.xml filled from identity.json, Cargo.toml's version and the
CPU. OUT/identity.txt says which identity was used. While identity.json
still has a TODO, the `test` identity is used and the run says so (exit
code 0; the Store would refuse such a package).
"""
import argparse
import json
import os
import re
import shutil
import sys
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
# release asset -> MSIX ProcessorArchitecture
EXES = {"bahth.exe": "x64", "bahth-x86.exe": "x86", "bahth-arm64.exe": "arm64"}
FIELDS = ("name", "publisher", "publisherDisplayName", "displayName")


def cargo_version():
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        m = re.search(r'^version\s*=\s*"([^"]+)"', f.read(), re.M)
    return m.group(1)


def msix_version(v):
    """0.9.1 -> 0.9.1.0. The Store keeps the fourth part for itself (0)."""
    parts = v.split(".")
    if not 1 <= len(parts) <= 3 or not all(p.isdigit() and int(p) <= 65535 for p in parts):
        raise ValueError(f"version {v!r} is not X.Y.Z with numbers up to 65535")
    return ".".join(parts + ["0"] * (4 - len(parts)))


def check_identity(ident):
    name, pub = ident["name"], ident["publisher"]
    if not re.fullmatch(r"[A-Za-z0-9.-]{3,50}", name):
        raise ValueError(f"identity name {name!r}: 3-50 letters, digits, '.' or '-'")
    if not re.fullmatch(r"CN=[^,]+(,\s*[A-Z]+=[^,]+)*", pub):
        raise ValueError(f"publisher {pub!r} must look like CN=...")
    for k in ("publisherDisplayName", "displayName"):
        if not ident[k].strip() or len(ident[k]) > 256:
            raise ValueError(f"{k} must be 1-256 characters")


def pick_identity(path):
    """(identity, is_test)."""
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    real = {k: data.get(k, "") for k in FIELDS}
    if any("TODO" in v or not v for v in real.values()):
        test = {k: data["test"][k] for k in FIELDS}
        check_identity(test)
        return test, True
    check_identity(real)
    return real, False


def fill(template, values):
    out = template
    for k, v in values.items():
        out = out.replace("{{" + k + "}}", escape(v, {'"': "&quot;"}))
    left = re.findall(r"\{\{[A-Z_]+\}\}", out)
    if left:
        raise ValueError(f"unfilled placeholders: {left}")
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--exes", required=True, help="folder with bahth.exe, bahth-x86.exe, bahth-arm64.exe")
    ap.add_argument("--out", required=True)
    ap.add_argument("--version", default=None, help="X.Y.Z (default: Cargo.toml)")
    ap.add_argument("--identity", default=os.path.join(HERE, "identity.json"))
    ap.add_argument("--require-all", action="store_true", help="fail unless all three exes are there")
    args = ap.parse_args(argv)

    version = msix_version((args.version or cargo_version()).lstrip("v"))
    ident, is_test = pick_identity(args.identity)
    with open(os.path.join(HERE, "AppxManifest.xml"), encoding="utf-8") as f:
        template = f.read()

    os.makedirs(args.out, exist_ok=True)
    made = []
    for exe, arch in EXES.items():
        src = os.path.join(args.exes, exe)
        if not os.path.isfile(src):
            if args.require_all:
                sys.exit(f"missing {src}")
            print(f"skip {arch}: no {exe}")
            continue
        with open(src, "rb") as f:
            if f.read(2) != b"MZ":
                sys.exit(f"{src} is not a Windows program")
        dest = os.path.join(args.out, arch)
        shutil.rmtree(dest, ignore_errors=True)
        shutil.copytree(os.path.join(HERE, "Assets"), os.path.join(dest, "Assets"))
        shutil.copyfile(src, os.path.join(dest, "bahth.exe"))
        # Licences: InstaFind's own and every crate's (packaging/make_notices.py);
        # About opens THIRD-PARTY-NOTICES.txt from the package folder.
        for doc in ("LICENSE", "THIRD-PARTY-NOTICES.txt"):
            shutil.copyfile(os.path.join(ROOT, doc), os.path.join(dest, doc + (".txt" if doc == "LICENSE" else "")))
        manifest = fill(template, {
            "IDENTITY_NAME": ident["name"],
            "PUBLISHER": ident["publisher"],
            "PUBLISHER_DISPLAY_NAME": ident["publisherDisplayName"],
            "DISPLAY_NAME": ident["displayName"],
            "VERSION": version,
            "ARCH": arch,
        })
        with open(os.path.join(dest, "AppxManifest.xml"), "w", encoding="utf-8", newline="\r\n") as f:
            f.write(manifest)
        made.append(arch)
    if not made:
        sys.exit("no exes found")

    kind = "TEST identity (identity.json still has a TODO; the Store will refuse this package)" if is_test else "Partner Center identity"
    summary = (
        f"version={version}\narchs={' '.join(made)}\ntest={'true' if is_test else 'false'}\n"
        f"name={ident['name']}\npublisher={ident['publisher']}\n"
    )
    with open(os.path.join(args.out, "identity.txt"), "w", encoding="utf-8") as f:
        f.write(summary)
    print(f"MSIX layouts for {', '.join(made)}, version {version}, {kind}: {ident['name']} / {ident['publisher']}")
    if is_test:
        print("::warning::identity.json still has a TODO: built with the TEST identity, not for upload.")


if __name__ == "__main__":
    main()
