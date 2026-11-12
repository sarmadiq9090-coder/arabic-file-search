#!/usr/bin/env python3
"""Fails if an InstaFind exe imports a DLL that is not part of Windows, or
does not tell Windows to take the DLLs it imports from System32 only.

    python ci/check_imports.py [--mingw-ok] bahth.exe [bahth-x86.exe bahth-arm64.exe ...]

Reads the PE headers, the import and delay-load import tables and the load
configuration with the standard library only (no dumpbin needed).

1. Imports: prints every DLL. Each must be on ALLOWED (or match an
   ALLOWED_PREFIXES entry). A C runtime DLL must never appear: not the
   Visual C++ runtime (VCRUNTIME*, MSVCP*, the UCRT api-ms-win-crt-*
   forwarders, ucrtbase), which is not installed on every PC, so the exe would
   not start there, and not msvcrt.dll, which only a mingw build imports. The
   exes are built with MSVC and .cargo/config.toml links the C runtime
   statically (+crt-static) to keep it out. GitHub's Windows runners have the
   runtime installed, so only this check catches a regression.

2. DLL search: the load configuration's DependentLoadFlags must be exactly
   0x800 (LOAD_LIBRARY_SEARCH_SYSTEM32; build.rs passes
   /DEPENDENTLOADFLAG:0x800 to the MSVC linker). Most of the allowed DLLs
   (version.dll, winhttp.dll, dbghelp.dll...) are not "known DLLs": without
   the flag Windows looks for them in the exe's own folder first, and the exe
   is often run from Downloads, where a DLL of that name could be planted.

3. Exploit mitigations (build.rs passes them explicitly): the PE header must
   mark the exe ASLR-compatible (DYNAMIC_BASE, with its relocations kept),
   DEP-compatible (NX_COMPAT), Control Flow Guard (GUARD_CF) and, for a 64-bit
   exe, HIGH_ENTROPY_VA.

build.yml runs this on each exe it publishes (jobs `exe` and `exe-arm64`),
without --mingw-ok. With --mingw-ok, an exe linked by GNU ld (a local mingw
cross-build: it imports msvcrt.dll and has no DependentLoadFlags, GNU ld has
no such option) only gets notes for those points and a missing mitigation;
every other finding still fails. Based on Lemon's tools/check_imports.py.
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
    "oleaut32.dll", "powrprof.dll", "propsys.dll", "psapi.dll", "rpcrt4.dll", "secur32.dll",
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


class Pe:
    """The parts of a PE file's headers the checks read."""

    def __init__(self, path):
        with open(path, "rb") as f:
            self.data = data = f.read()
        if data[:2] != b"MZ":
            raise ValueError("not a PE file (no MZ header)")
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        if data[pe:pe + 4] != b"PE\0\0":
            raise ValueError("not a PE file (no PE signature)")
        n_sections, = struct.unpack_from("<H", data, pe + 6)
        # IMAGE_FILE_HEADER.Characteristics and IMAGE_OPTIONAL_HEADER.DllCharacteristics.
        self.characteristics, = struct.unpack_from("<H", data, pe + 22)
        opt_size, = struct.unpack_from("<H", data, pe + 20)
        opt = pe + 24
        magic, = struct.unpack_from("<H", data, opt)
        self.pe32_plus = magic == 0x20B  # PE32+ (x64, ARM64) / PE32 (x86)
        self.dll_characteristics, = struct.unpack_from("<H", data, opt + 70)
        # MajorLinkerVersion: 14 for MSVC's link.exe, 2 for GNU ld (mingw).
        self.linker_major = data[opt + 2]
        self.dirs = opt + (112 if self.pe32_plus else 96)
        self.sections = []
        sec = opt + opt_size
        for i in range(n_sections):
            vsize, vaddr, raw_size, raw_ptr = struct.unpack_from("<IIII", data, sec + i * 40 + 8)
            self.sections.append((vaddr, vsize, raw_ptr, raw_size))

    def directory(self, index):
        """(RVA, size) of a data directory."""
        return struct.unpack_from("<II", self.data, self.dirs + index * 8)


# Offset of DependentLoadFlags (a WORD) in IMAGE_LOAD_CONFIG_DIRECTORY64 / 32.
DEPENDENT_LOAD_FLAGS_AT = {True: 78, False: 54}
LOAD_LIBRARY_SEARCH_SYSTEM32 = 0x800


IMAGE_FILE_RELOCS_STRIPPED = 0x0001
# (DllCharacteristics bit, name, what it does, 64-bit exes only)
MITIGATIONS = (
    (0x0040, "DYNAMIC_BASE", "ASLR: loaded at a random address (/DYNAMICBASE)", False),
    (0x0100, "NX_COMPAT", "DEP: data never runs as code (/NXCOMPAT)", False),
    (0x0020, "HIGH_ENTROPY_VA", "ASLR over the whole 64-bit address space (/HIGHENTROPYVA)", True),
    (0x4000, "GUARD_CF", "Control Flow Guard (-C control-flow-guard in .cargo/config.toml)", False),
)


def missing_mitigations(pe):
    """The mitigations the PE header does not have, as (name, what) pairs."""
    missing = [(name, what) for bit, name, what, only64 in MITIGATIONS
               if (pe.pe32_plus or not only64) and not pe.dll_characteristics & bit]
    if pe.characteristics & IMAGE_FILE_RELOCS_STRIPPED:
        missing.append(("relocations", "stripped by /FIXED, so ASLR cannot move the exe"))
    return missing


def dependent_load_flags(pe):
    """DependentLoadFlags of the load configuration (data directory 10), or
    None when the exe has no load configuration or one too old to have it."""
    rva, _ = pe.directory(10)
    if rva == 0:
        return None
    off = rva_to_offset(pe.sections, rva)
    # The structure's own first field is its size (the directory's size
    # field is not reliable: older linkers wrote a fixed 0x40 there).
    size, = struct.unpack_from("<I", pe.data, off)
    at = DEPENDENT_LOAD_FLAGS_AT[pe.pe32_plus]
    if size < at + 2:
        return None
    return struct.unpack_from("<H", pe.data, off + at)[0]


def imported_dlls(pe):
    data, sections, dirs = pe.data, pe.sections, pe.dirs
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


def check(path, mingw_ok=False):
    print(f"{path}:")
    pe = Pe(path)
    # A local mingw cross-build, tolerated only when asked for.
    mingw = mingw_ok and pe.linker_major < 14
    bad = []
    for name, kind in imported_dlls(pe):
        low = name.lower()
        if mingw and low == "msvcrt.dll":
            verdict = "note: mingw build (never published)"
        elif low.startswith(FORBIDDEN_PREFIXES):
            verdict = "FORBIDDEN (C runtime DLL)"
        elif low in ALLOWED or low.startswith(ALLOWED_PREFIXES):
            verdict = "ok"
        else:
            verdict = "NOT ALLOWED (not a Windows system DLL)"
        print(f"  {name:<40} {kind:<10} {verdict}")
        if verdict != "ok" and not verdict.startswith("note"):
            bad.append(name)
    ok = True
    if bad:
        print(f"::error::{path} imports a C runtime DLL or a DLL that is not part of Windows: "
              + ", ".join(bad)
              + ". The exe may fail to start on PCs without them. See ci/check_imports.py.")
        ok = False
    else:
        print("  All imported DLLs are Windows system DLLs.")

    flags = dependent_load_flags(pe)
    shown = "missing (no load configuration with that field)" if flags is None else f"{flags:#06x}"
    if flags == LOAD_LIBRARY_SEARCH_SYSTEM32:
        print(f"  DependentLoadFlags {shown}: imported DLLs load from System32 only.")
    elif mingw:
        print(f"  DependentLoadFlags {shown}: note: mingw build (never published); the MSVC exes must have 0x0800.")
    else:
        print(f"::error::{path}: DependentLoadFlags is {shown}, expected 0x0800 (LOAD_LIBRARY_SEARCH_SYSTEM32). "
              "Windows would look for the imported DLLs in the exe's own folder first, so a DLL planted "
              "beside the exe (in Downloads, say) would be loaded. build.rs must pass "
              "/DEPENDENTLOADFLAG:0x800 to the linker. See ci/check_imports.py.")
        ok = False

    missing = missing_mitigations(pe)
    if not missing:
        print("  Mitigations: " + ", ".join(name for bit, name, what, only64 in MITIGATIONS
                                            if pe.pe32_plus or not only64) + ", relocations kept.")
    for name, what in missing:
        if mingw:
            print(f"  {name} missing: note: mingw build (never published).")
        else:
            print(f"::error::{path}: the PE header lacks {name} ({what}). build.rs or .cargo/config.toml must turn it on. "
                  "See ci/check_imports.py.")
            ok = False
    return ok


def main():
    args = sys.argv[1:]
    mingw_ok = "--mingw-ok" in args
    paths = [a for a in args if a != "--mingw-ok"]
    if not paths:
        print(__doc__.strip().splitlines()[3].strip())
        return 2
    results = [check(p, mingw_ok) for p in paths]
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
