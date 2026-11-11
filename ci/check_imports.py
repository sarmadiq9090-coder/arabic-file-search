#!/usr/bin/env python3
"""Fails if an InstaFind exe imports a DLL that is not part of Windows.

    python ci/check_imports.py bahth.exe [bahth-x86.exe bahth-arm64.exe ...]

Reads the PE import and delay-load import tables with the standard library
only (no dumpbin needed) and prints every DLL. Each must be on ALLOWED (or
match an ALLOWED_PREFIXES entry). A C runtime DLL must never appear: not the
Visual C++ runtime (VCRUNTIME*, MSVCP*, the UCRT api-ms-win-crt-* forwarders,
ucrtbase), which is not installed on every PC, so the exe would not start
there, and not msvcrt.dll, which only a mingw build imports. The exes are
built with MSVC and .cargo/config.toml links the C runtime statically
(+crt-static) to keep it out. GitHub's Windows runners have the runtime
installed, so only this check catches a regression; build.yml runs it on each
exe it publishes (jobs `exe` and `exe-arm64`). Same check as Lemon's
tools/check_imports.py.
"""
import struct
import sys

# DLLs that ship with every supported Windows version (10/11). Lower case.
# usp10.dll is Uniscribe (Arabic text shaping, src/draw.rs).
ALLOWED = {
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "combase.dll",
    "comctl32.dll", "comdlg32.dll", "crypt32.dll", "d2d1.dll", "d3d11.dll",
    "dbghelp.dll", "dwmapi.dll", "dwrite.dll", "dxgi.dll", "gdi32.dll",
    "imm32.dll", "iphlpapi.dll", "kernel32.dll", "kernelbase.dll",
    "msimg32.dll", "mswsock.dll", "ncrypt.dll", "ntdll.dll", "ole32.dll",
    "oleaut32.dll", "powrprof.dll", "propsys.dll", "rpcrt4.dll", "secur32.dll",
    "setupapi.dll", "shcore.dll", "shell32.dll", "shlwapi.dll",
    "uiautomationcore.dll", "urlmon.dll", "user32.dll", "userenv.dll",
    "usp10.dll", "uxtheme.dll", "version.dll", "winhttp.dll", "wininet.dll", "winmm.dll",
    "ws2_32.dll",
}
# Windows API sets (api-ms-win-core-*, ...). api-ms-win-crt-* is the UCRT and
# is rejected below before this list is consulted.
ALLOWED_PREFIXES = ("api-ms-win-core-", "api-ms-win-security-", "api-ms-win-shcore-")
FORBIDDEN_PREFIXES = ("vcruntime", "msvcp", "msvcr", "ucrtbase", "api-ms-win-crt-", "concrt", "vccorlib")


def rva_to_offset(sections, rva):
    for vaddr, vsize, raw_ptr, raw_size in sections:
        if vaddr <= rva < vaddr + max(vsize, raw_size):
            return raw_ptr + (rva - vaddr)
    raise ValueError(f"RVA {rva:#x} is outside every section")


def cstr(data, off):
    end = data.index(b"\0", off)
    return data[off:end].decode("ascii", "replace")


def imported_dlls(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:2] != b"MZ":
        raise ValueError("not a PE file (no MZ header)")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("not a PE file (no PE signature)")
    n_sections, = struct.unpack_from("<H", data, pe + 6)
    opt_size, = struct.unpack_from("<H", data, pe + 20)
    opt = pe + 24
    magic, = struct.unpack_from("<H", data, opt)
    dirs = opt + (112 if magic == 0x20B else 96)  # PE32+ / PE32
    sections = []
    sec = opt + opt_size
    for i in range(n_sections):
        vsize, vaddr, raw_size, raw_ptr = struct.unpack_from("<IIII", data, sec + i * 40 + 8)
        sections.append((vaddr, vsize, raw_ptr, raw_size))

    found = []
    # Data directory 1 = imports (20-byte descriptors, name RVA at +12),
    # 13 = delay-load imports (32-byte descriptors, name RVA at +4).
    for index, size, name_at, kind in ((1, 20, 12, "import"), (13, 32, 4, "delay-load")):
        rva, _ = struct.unpack_from("<II", data, dirs + index * 8)
        if rva == 0:
            continue
        off = rva_to_offset(sections, rva)
        while True:
            desc = data[off:off + size]
            if len(desc) < size or desc == b"\0" * size:
                break
            name_rva, = struct.unpack_from("<I", desc, name_at)
            found.append((cstr(data, rva_to_offset(sections, name_rva)), kind))
            off += size
    return found


def check(path):
    print(f"{path}:")
    bad = []
    for name, kind in imported_dlls(path):
        low = name.lower()
        if low.startswith(FORBIDDEN_PREFIXES):
            verdict = "FORBIDDEN (C runtime DLL)"
        elif low in ALLOWED or low.startswith(ALLOWED_PREFIXES):
            verdict = "ok"
        else:
            verdict = "NOT ALLOWED (not a Windows system DLL)"
        print(f"  {name:<40} {kind:<10} {verdict}")
        if verdict != "ok":
            bad.append(name)
    if bad:
        print(f"::error::{path} imports a C runtime DLL or a DLL that is not part of Windows: "
              + ", ".join(bad)
              + ". The exe may fail to start on PCs without them. See ci/check_imports.py.")
        return False
    print("  All imported DLLs are Windows system DLLs.")
    return True


def main():
    if len(sys.argv) < 2:
        print(__doc__.strip().splitlines()[2].strip())
        return 2
    results = [check(p) for p in sys.argv[1:]]
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
