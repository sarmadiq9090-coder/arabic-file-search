# -*- coding: utf-8 -*-
"""Flow Launcher plugin for بحث سريع (Bahth Sari').

Flow runs `python main.py '<json request>'` and reads one JSON-RPC v1 reply
from stdout. The search itself is done by bahth.exe's command-line mode:
`bahth.exe -s "<query>" --json -n 30`, which prints a JSON array of
{path, name, size, modified, is_dir} and exits 0 (results), 1 (none) or
2 (error, usually no index yet).

Standard library only, so it runs on Flow's bundled Python as is.
"""

import json
import os
import subprocess
import sys

ICON = "Images\\bahth.png"
MAX_RESULTS = 30
TIMEOUT = 3  # seconds
ENV_VAR = "BAHTH_EXE"
# Windows only; 0 elsewhere so the tests run on Linux too.
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000 if os.name == "nt" else 0)

EXIT_FOUND, EXIT_NONE, EXIT_ERROR = 0, 1, 2


def default_exe():
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~\\AppData\\Local")
    return os.path.join(base, "Programs", "Bahth", "bahth.exe")


def find_exe(settings=None):
    """The exe to run: plugin setting, then the BAHTH_EXE env var, then the default install path."""
    for candidate in ((settings or {}).get("bahth_path"), os.environ.get(ENV_VAR)):
        if candidate and candidate.strip():
            return os.path.expandvars(candidate.strip().strip('"'))
    return default_exe()


def message(title, subtitle=""):
    return {"Title": title, "SubTitle": subtitle, "IcoPath": ICON}


def human_size(n):
    if n is None:
        return ""
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024.0
    return ""


def run_bahth(exe, query):
    """Returns (exit_code, items, error_text)."""
    try:
        proc = subprocess.run(
            [exe, "-s", query, "--json", "-n", str(MAX_RESULTS)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=TIMEOUT,
            creationflags=CREATE_NO_WINDOW,
        )
    except FileNotFoundError:
        return EXIT_ERROR, [], "not-found"
    except subprocess.TimeoutExpired:
        return EXIT_ERROR, [], "timeout"
    except OSError as e:
        return EXIT_ERROR, [], str(e)
    out = proc.stdout.decode("utf-8", errors="replace").lstrip("﻿")
    err = proc.stderr.decode("utf-8", errors="replace").strip()
    if proc.returncode == EXIT_ERROR:
        return EXIT_ERROR, [], err
    try:
        items = json.loads(out) if out.strip() else []
    except ValueError:
        return EXIT_ERROR, [], "bad output"
    return proc.returncode, items if isinstance(items, list) else [], ""


def query(text, settings=None):
    text = (text or "").strip()
    if not text:
        return [message("بحث سريع — اكتب كلمة للبحث", "Bahth Sari': type something to search")]
    exe = find_exe(settings)
    if not os.path.isfile(exe):
        return [message(
            "لم يُعثر على bahth.exe — bahth.exe not found",
            f"{exe} — حدّد المسار في إعدادات الإضافة أو متغير {ENV_VAR} / set the path in plugin settings or {ENV_VAR}",
        )]
    code, items, err = run_bahth(exe, text)
    if code == EXIT_ERROR:
        if err == "timeout":
            return [message("انتهت المهلة — search timed out", "حاول بكلمات أدق / try a more specific query")]
        return [message(
            "لا يوجد فهرس: افتح «بحث سريع» مرة لبنائه",
            "No index yet: open Bahth Sari' once to build it" + (f" ({err.splitlines()[0]})" if err and err != "not-found" else ""),
        )]
    if not items:
        return [message("لا نتائج", f"No results for: {text}")]
    results = []
    for i, it in enumerate(items):
        path = it.get("path", "")
        if not path:
            continue
        name = it.get("name") or os.path.basename(path.rstrip("\\")) or path
        extra = "مجلد" if it.get("is_dir") else human_size(it.get("size"))
        results.append({
            "Title": name,
            "SubTitle": path + (f"  ·  {extra}" if extra else ""),
            "IcoPath": ICON,
            "Score": MAX_RESULTS - i,  # keep bahth's own ranking
            "ContextData": path,
            "JsonRPCAction": {"method": "open_file", "parameters": [path], "dontHideAfterAction": False},
        })
    return results


def context_menu(path):
    if isinstance(path, list):
        path = path[0] if path else ""
    return [
        {
            "Title": "فتح المجلد — Open containing folder",
            "SubTitle": path,
            "IcoPath": ICON,
            "JsonRPCAction": {"method": "open_folder", "parameters": [path]},
        },
        {
            "Title": "فتح — Open",
            "SubTitle": path,
            "IcoPath": ICON,
            "JsonRPCAction": {"method": "open_file", "parameters": [path]},
        },
    ]


def open_file(path):
    os.startfile(path)  # Windows only


def open_folder(path):
    # explorer wants `/select,"path"` as one argument; a list would re-quote it.
    subprocess.Popen(f'explorer /select,"{path}"', creationflags=CREATE_NO_WINDOW)


def handle(request):
    """Dispatches one JSON-RPC request; returns the reply dict or None for actions."""
    method = request.get("method", "")
    params = request.get("parameters") or []
    settings = request.get("settings") or {}
    if method == "query":
        return {"result": query(params[0] if params else "", settings)}
    if method == "context_menu":
        return {"result": context_menu(params[0] if params else "")}
    if method in ("open_file", "open_folder") and params:
        (open_file if method == "open_file" else open_folder)(params[0])
        return None
    return {"result": []}


def main(argv):
    try:
        request = json.loads(argv[1]) if len(argv) > 1 else {}
    except ValueError:
        request = {}
    reply = handle(request)
    if reply is not None:
        # ASCII-only JSON, so the console code page cannot mangle Arabic.
        sys.stdout.write(json.dumps(reply, ensure_ascii=True))
        sys.stdout.flush()


if __name__ == "__main__":
    main(sys.argv)
