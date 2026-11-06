#!/usr/bin/env python3
"""Writes the Scoop, winget and Chocolatey manifests for one release.

This script is the single source of truth for the package metadata: the CI
(`release` job in build.yml and packaging.yml) runs it after every release,
and it rewrites these files from the templates below:

    bucket/instafind.json
    packaging/winget/manifests/s/Sarmad/InstaFind/<version>/*.yaml
    packaging/chocolatey/instafind.nuspec
    packaging/chocolatey/tools/chocolateyinstall.ps1

To change a description, tag or URL, edit it here and rerun the script, not
the generated files. Older winget version folders are removed so the folder
always holds exactly the manifest to submit.

Usage:
    python3 packaging/update-manifests.py 0.6.0 <sha256 of bahth.exe> 2026-11-01
    python3 packaging/update-manifests.py v0.7.0 <sha256> <date> \
        --sha-x86 <sha256 of bahth-x86.exe> --sha-arm64 <sha256 of bahth-arm64.exe>
    python3 packaging/update-manifests.py v0.6.0 <sha256> <date> --root /some/copy

Since 0.7.0 every release has three exes: bahth.exe (x64), bahth-x86.exe and
bahth-arm64.exe. Pass the hashes of the ones the release has; a manifest only
lists the CPUs whose hash was given (x64 is always required).

Standard library only.
"""

import argparse
import json
import re
import shutil
import sys
from pathlib import Path

REPO = "https://github.com/sarmadiq9090-coder/arabic-file-search"
OWNER_URL = "https://github.com/sarmadiq9090-coder"
ICON_URL = "https://cdn.jsdelivr.net/gh/sarmadiq9090-coder/arabic-file-search@main/assets/bahth-256.png"
WINGET_ID = "Sarmad.InstaFind"
WINGET_SCHEMA = "1.12.0"  # the version winget-pkgs currently recommends
PUBLISHER = "Sarmad"
COPYRIGHT = "Copyright (c) 2026 Saif Second"  # as in LICENSE

SHORT_EN = "Instant file search for Windows, in Arabic and English, even inside documents."
SHORT_AR = "أسرع طريقة لإيجاد أي ملف على ويندوز، بالعربية والإنجليزية، وحتى داخل المستندات."

DESC_EN = """\
InstaFind (بحث سريع) indexes all your drives once and shows results while you type, even with millions of files.
It understands Arabic the way people actually type it: hamza forms, ta marbuta, diacritics and the "ال" prefix are ignored, and text typed on the wrong keyboard layout or in Arabizi is still found.
It also searches inside documents (docx, xlsx, pptx, pdf, odt, rtf, e-mail, epub, source code), inside images and scanned PDFs through the OCR built into Windows, and inside zip/7z/tar archives.
Extras: duplicate finder, folder sizes, thumbnails, find & replace in text files, a global hotkey, a command line (bahth --help), and optional read-only search from your phone.
Single portable exe, no installer. Data lives in %LOCALAPPDATA%\\BahthSari. The interface is Arabic or English and follows Windows."""

DESC_AR = """\
برنامج مجاني ومفتوح المصدر يفهرس كل أقراصك مرة واحدة ثم يُظهر النتائج أثناء الكتابة، حتى مع ملايين الملفات.
يفهم العربية كما تُكتب فعلاً: «احمد» تجد «أحمد»، و«التقرير» تجد «تقرير.pdf»، والكلمة المكتوبة بلوحة المفاتيح الخطأ أو بالفرانكو تُوجد أيضاً.
يبحث داخل المستندات (docx وxlsx وpptx وpdf وodt وrtf والبريد وepub وملفات البرمجة)، وداخل الصور وملفات PDF الممسوحة ضوئياً بمحرك التعرف على النصوص في ويندوز، وداخل الملفات المضغوطة.
ومعه: الملفات المكررة، أحجام المجلدات، المصغرات، الاستبدال في عدة ملفات، اختصار عام، سطر الأوامر، والبحث من الهاتف.
ملف واحد لا يحتاج تثبيتاً، وبياناته في %LOCALAPPDATA%\\BahthSari، والواجهة عربية أو إنجليزية."""

TAGS = ["search", "file-search", "find", "arabic", "full-text-search", "ocr", "indexer", "desktop-search", "portable", "rust"]

SCOOP_NOTES = [
    "Installed with Scoop, InstaFind does not update itself (versions up to 0.9.0 still offer to):",
    "run 'scoop update instafind' to get new versions.",
    "Index, settings and log live in %LOCALAPPDATA%\\BahthSari (not in the Scoop folder);",
    "delete that folder and HKCU\\Software\\BahthSari yourself to remove all data.",
]


# CPU -> release asset. The x64 name is what the in-app updater of every
# installed copy looks for; it never changes.
ASSETS = {"x64": "bahth.exe", "x86": "bahth-x86.exe", "arm64": "bahth-arm64.exe"}
SCOOP_ARCH = {"x64": "64bit", "x86": "32bit", "arm64": "arm64"}


def exe_url(version, arch="x64"):
    return f"{REPO}/releases/download/v{version}/{ASSETS[arch]}"


def yaml_str(s):
    """A YAML double-quoted scalar (JSON strings are valid YAML)."""
    return json.dumps(s, ensure_ascii=False)


def yaml_block(s, indent="  "):
    return "|-\n" + "\n".join(indent + line if line else "" for line in s.splitlines())


def scoop(version, shas):
    m = {
        "version": version,
        "description": "InstaFind (بحث سريع): " + SHORT_EN,
        "homepage": REPO,
        "license": "MIT",
        "notes": SCOOP_NOTES,
        # Every CPU's exe is saved as bahth.exe (the `#/bahth.exe` suffix
        # renames the download) so bin and shortcuts are the same for all.
        "architecture": {
            SCOOP_ARCH[a]: {"url": exe_url(version, a) + ("" if a == "x64" else "#/bahth.exe"), "hash": h}
            for a, h in shas.items()
        },
        "bin": ["bahth.exe", ["bahth.exe", "instafind"]],
        "shortcuts": [["bahth.exe", "InstaFind"]],
        # The app turns on "Start with Windows" (HKCU Run value BahthSari) by
        # default. On uninstall (not on update) drop that value when it points
        # into this app's Scoop folder; user data is left alone.
        "uninstaller": {
            "script": [
                "if ($cmd -eq 'uninstall') {",
                "    $run = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run'",
                "    $value = (Get-ItemProperty -Path $run -Name 'BahthSari' -ErrorAction SilentlyContinue).BahthSari",
                "    if ($value -and $value.ToLower().Contains((Split-Path $dir).ToLower())) {",
                "        Remove-ItemProperty -Path $run -Name 'BahthSari' -ErrorAction SilentlyContinue",
                "    }",
                "}",
            ]
        },
        "checkver": "github",
        "autoupdate": {
            "architecture": {
                SCOOP_ARCH[a]: {
                    "url": f"{REPO}/releases/download/v$version/{ASSETS[a]}" + ("" if a == "x64" else "#/bahth.exe")
                }
                for a in shas
            }
        },
    }
    text = json.dumps(m, ensure_ascii=False, indent=4)
    # Scoop style: short arrays of strings on one line, e.g. ["bahth.exe", "instafind"].
    short = re.compile(r'\[\s*("[^"\n]*"(?:,\s*"[^"\n]*")*)\s*\]')
    one_line = lambda mo: json.dumps(json.loads(mo.group(0)), ensure_ascii=False)
    text = short.sub(lambda mo: one_line(mo) if len(one_line(mo)) < 60 else mo.group(0), text)
    return text + "\n"


def winget_header(kind):
    return (
        "# Generated by packaging/update-manifests.py\n"
        f"# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{WINGET_SCHEMA}.schema.json\n\n"
    )


def winget(version, shas, date):
    common = f"PackageIdentifier: {WINGET_ID}\nPackageVersion: {version}\n"
    tail = lambda t: f"ManifestType: {t}\nManifestVersion: {WINGET_SCHEMA}\n"
    tags = "".join(f"- {t}\n" for t in TAGS)
    files = {}
    files[f"{WINGET_ID}.yaml"] = (
        winget_header("version") + common + "DefaultLocale: en-US\n" + tail("version")
    )
    files[f"{WINGET_ID}.installer.yaml"] = (
        winget_header("installer")
        + common
        + "InstallerType: portable\n"
        + "Commands:\n- bahth\n"
        + "MinimumOSVersion: 10.0.0.0\n"
        + f"ReleaseDate: {date}\n"
        + "Installers:\n"
        + "".join(
            f"- Architecture: {a}\n"
            f"  InstallerUrl: {exe_url(version, a)}\n"
            f"  InstallerSha256: {h.upper()}\n"
            for a, h in shas.items()
        )
        + tail("installer")
    )
    files[f"{WINGET_ID}.locale.en-US.yaml"] = (
        winget_header("defaultLocale")
        + common
        + "PackageLocale: en-US\n"
        + f"Publisher: {PUBLISHER}\n"
        + f"PublisherUrl: {OWNER_URL}\n"
        + f"PublisherSupportUrl: {REPO}/issues\n"
        + f"Author: {PUBLISHER}\n"
        + "PackageName: InstaFind\n"
        + f"PackageUrl: {REPO}\n"
        + "License: MIT\n"
        + f"LicenseUrl: {REPO}/blob/main/LICENSE\n"
        + f"Copyright: {yaml_str(COPYRIGHT)}\n"
        + f"ShortDescription: {yaml_str(SHORT_EN)}\n"
        + f"Description: {yaml_block(DESC_EN)}\n"
        + "Moniker: instafind\n"
        + "Tags:\n" + tags
        + f"ReleaseNotesUrl: {REPO}/releases/tag/v{version}\n"
        + "Documentations:\n"
        + "- DocumentLabel: README\n"
        + f"  DocumentUrl: {REPO}#english\n"
        + tail("defaultLocale")
    )
    files[f"{WINGET_ID}.locale.ar-SA.yaml"] = (
        winget_header("locale")
        + common
        + "PackageLocale: ar-SA\n"
        + f"Publisher: {PUBLISHER}\n"
        + f"PublisherUrl: {OWNER_URL}\n"
        + f"PublisherSupportUrl: {REPO}/issues\n"
        + f"Author: {PUBLISHER}\n"
        + f"PackageName: {yaml_str('InstaFind — بحث سريع')}\n"
        + f"PackageUrl: {REPO}\n"
        + "License: MIT\n"
        + f"LicenseUrl: {REPO}/blob/main/LICENSE\n"
        + f"Copyright: {yaml_str(COPYRIGHT)}\n"
        + f"ShortDescription: {yaml_str(SHORT_AR)}\n"
        + f"Description: {yaml_block(DESC_AR)}\n"
        + "Tags:\n" + "".join(f"- {t}\n" for t in ["بحث", "ملفات", "عربي"] + TAGS[:7])
        + f"ReleaseNotesUrl: {REPO}/releases/tag/v{version}\n"
        + tail("locale")
    )
    return files


def nuspec(version):
    xml_esc = lambda s: s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    return f"""\
<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by packaging/update-manifests.py; edit the template there. -->
<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
  <metadata>
    <id>instafind</id>
    <version>{version}</version>
    <packageSourceUrl>{REPO}/tree/main/packaging/chocolatey</packageSourceUrl>
    <owners>{PUBLISHER}</owners>
    <title>InstaFind — بحث سريع</title>
    <authors>{PUBLISHER}</authors>
    <projectUrl>{REPO}</projectUrl>
    <iconUrl>{ICON_URL}</iconUrl>
    <copyright>{COPYRIGHT}</copyright>
    <licenseUrl>{REPO}/blob/main/LICENSE</licenseUrl>
    <requireLicenseAcceptance>false</requireLicenseAcceptance>
    <projectSourceUrl>{REPO}</projectSourceUrl>
    <docsUrl>{REPO}#english</docsUrl>
    <bugTrackerUrl>{REPO}/issues</bugTrackerUrl>
    <tags>{" ".join(["instafind", "bahth"] + TAGS)}</tags>
    <summary>{xml_esc(SHORT_EN)}</summary>
    <description><![CDATA[{DESC_EN}

## Notes
- The package downloads the single portable `bahth.exe` from the GitHub release and adds the `bahth` command and a Start menu shortcut.
- Use `choco upgrade instafind` to update. The app's own update prompt cannot replace the exe in the Chocolatey folder without administrator rights.
- Uninstalling leaves the user data in `%LOCALAPPDATA%\\BahthSari` and `HKCU\\Software\\BahthSari`.
]]></description>
    <releaseNotes>{REPO}/releases/tag/v{version}</releaseNotes>
  </metadata>
  <files>
    <file src="tools\\**" target="tools" />
  </files>
</package>
"""


def choco_install(version, shas):
    # 32-bit Windows gets bahth-x86.exe; Windows on ARM runs the x64 exe
    # (Chocolatey has no ARM64 URL parameter).
    x86 = (
        f"  url            = '{exe_url(version, 'x86')}'\n"
        f"  checksum       = '{shas['x86']}'\n"
        "  checksumType   = 'sha256'\n"
    ) if "x86" in shas else ""
    return f"""\
# Generated by packaging/update-manifests.py; edit the template there.
$ErrorActionPreference = 'Stop'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$exe = Join-Path $toolsDir 'bahth.exe'

$packageArgs = @{{
  packageName    = $env:ChocolateyPackageName
  fileFullPath   = $exe
  url64bit       = '{exe_url(version)}'
  checksum64     = '{shas["x64"]}'
  checksumType64 = 'sha256'
{x86}}}
Get-ChocolateyWebFile @packageArgs

# Chocolatey shims bahth.exe as `bahth` automatically.

$programs = [Environment]::GetFolderPath('CommonPrograms')
Install-ChocolateyShortcut -ShortcutFilePath (Join-Path $programs 'InstaFind.lnk') `
  -TargetPath $exe -WorkingDirectory $toolsDir -Description 'InstaFind — بحث سريع'
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("version", help="release version, e.g. 0.6.0 or v0.6.0")
    ap.add_argument("sha256", help="SHA-256 of the release's bahth.exe (x64)")
    ap.add_argument("--sha-x86", help="SHA-256 of bahth-x86.exe, if the release has it")
    ap.add_argument("--sha-arm64", help="SHA-256 of bahth-arm64.exe, if the release has it")
    ap.add_argument("date", help="release date, YYYY-MM-DD")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="repository root to write into (default: this script's repo)")
    ap.add_argument("--allow-older", action="store_true",
                    help="write even if the bucket already holds a newer version")
    a = ap.parse_args()

    version = a.version.lstrip("vV")
    shas = {"x64": a.sha256, "x86": a.sha_x86, "arm64": a.sha_arm64}
    shas = {k: v.strip().lower() for k, v in shas.items() if v and v.strip()}
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        sys.exit(f"bad version: {a.version!r} (expected X.Y.Z)")
    if "x64" not in shas:
        sys.exit("the sha256 of bahth.exe (x64) is required")
    for arch, sha in shas.items():
        if not re.fullmatch(r"[0-9a-f]{64}", sha):
            sys.exit(f"bad sha256 for {arch}: {sha!r}")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", a.date):
        sys.exit(f"bad date: {a.date!r} (expected YYYY-MM-DD)")

    root = a.root.resolve()
    current = root / "bucket/instafind.json"
    if current.is_file() and not a.allow_older:
        m = re.search(r'"version":\s*"(\d+)\.(\d+)\.(\d+)"', current.read_text(encoding="utf-8"))
        new = tuple(int(x) for x in version.split("."))
        if m and tuple(int(x) for x in m.groups()) > new:
            sys.exit(f"the manifests already hold {'.'.join(m.groups())}, newer than {version}; "
                     "pass --allow-older to roll them back")
    written = []

    def write(rel, text):
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        # UTF-8 without BOM and LF endings for JSON/YAML/XML. The .ps1 gets a
        # BOM so Windows PowerShell 5.1 reads its non-ASCII text correctly.
        data = text.encode("utf-8")
        if rel.endswith(".ps1"):
            data = b"\xef\xbb\xbf" + data
        p.write_bytes(data)
        written.append(rel)

    write("bucket/instafind.json", scoop(version, shas))

    wroot = root / "packaging/winget/manifests/s/Sarmad/InstaFind"
    if wroot.is_dir():
        for old in wroot.iterdir():
            if old.is_dir() and old.name != version:
                shutil.rmtree(old)
    for name, text in winget(version, shas, a.date).items():
        write(f"packaging/winget/manifests/s/Sarmad/InstaFind/{version}/{name}", text)

    write("packaging/chocolatey/instafind.nuspec", nuspec(version))
    write("packaging/chocolatey/tools/chocolateyinstall.ps1", choco_install(version, shas))

    for rel in written:
        print("wrote", rel)


if __name__ == "__main__":
    main()
