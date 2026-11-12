#!/usr/bin/env python3
"""Turns the `mem-test` entry of ci/test-plan.json into the job matrix of
.github/workflows/mem-test.yml (its `setup` job runs this).

    {"mem-test": {"exes": ["0.9.1", "0.10.0", "branch"],
                  "scenarios": ["A", "B", "C", "D", "E", "F", "G", "H", "O", "K", "M", "S", "Q"],
                  "quick": ["B", "C", "E", "F"]}}

exes       which exes to measure: the released 0.9.1 and 0.10.0, and "branch",
           the exe built from the checked-out branch.
scenarios  which scenarios of ci/mem-test.ps1 to run in full.
quick      optional: scenarios for one short run (waits and data at 5 %) that
           only shows early whether the harness works; its numbers stay out
           of the report's tables. Never D (it uses the network).
Optional, for the branch build's test hook and the council's criteria:
    "save_delay_var": "BAHTH_TEST_SAVE_DELAY_MS"   the hook's environment variable
    "save_delay_ms": 90000                         its value in scenario H
    "fail_on_criteria": true                       a criterion the branch build fails fails the leg

What is not in the plan does not run. On a push, no file, no `mem-test` entry
or an empty one means: run nothing. Started by hand (workflow_dispatch)
without an entry, everything runs.

The legs (one Windows job each, at most three at a time):
    quick          the quick scenarios on one exe
    churn-<exe>    A, F (and G for the branch exe)
    hook-branch    H (the save-delay hook: only the branch build has it)
    robust-<exe>   O, K, M, S, Q
    update         D (needs 0.9.1 and 0.10.0) and G for 0.10.0
    static-<exe>   B, C, E
Prints the plan and writes `matrix`, `any`, `branch` and `released` to
$GITHUB_OUTPUT.
"""
import json
import os
import re
import sys

EXES = ["0.10.0", "branch", "0.9.1"]  # the order the legs start in
SCENARIOS = ["A", "B", "C", "D", "E", "F", "G", "H", "O", "K", "M", "S", "Q"]
RELEASED = ("0.9.1", "0.10.0")


def read_options(entry):
    """The optional settings of the entry, checked, as strings for the matrix."""
    var = str(entry.get("save_delay_var") or "BAHTH_TEST_SAVE_DELAY_MS")
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,60}", var):
        print(f"::warning::ci/test-plan.json: mem-test.save_delay_var {var!r} is not a variable name; the default is used")
        var = "BAHTH_TEST_SAVE_DELAY_MS"
    try:
        ms = max(0, min(600000, int(entry.get("save_delay_ms", 90000))))
    except (TypeError, ValueError):
        ms = 90000
    return {"save_delay_var": var, "save_delay_ms": str(ms), "fail_on_criteria": "true" if entry.get("fail_on_criteria") is True else "false"}


def read_plan(path, event):
    """The `mem-test` entry as (exes, scenarios, quick, options)."""
    entry = None
    try:
        with open(path, encoding="utf-8-sig") as f:
            data = json.load(f)
        if isinstance(data, dict):
            entry = data.get("mem-test")
    except FileNotFoundError:
        print(f"{path}: no such file")
    except (OSError, ValueError) as e:
        print(f"::warning::{path} could not be read: {e}")
    if not isinstance(entry, dict) or not entry:
        if event == "workflow_dispatch":
            print("no `mem-test` entry in the plan; started by hand: everything runs")
            return list(EXES), list(SCENARIOS), [], read_options({})
        print("no `mem-test` entry in the plan: nothing to run")
        return [], [], [], read_options({})

    def names(key, allowed):
        raw = entry.get(key) or []
        if isinstance(raw, str):
            raw = raw.split(",")
        wanted = [str(x).strip() for x in raw]
        for x in wanted:
            if x.upper() not in [a.upper() for a in allowed]:
                print(f"::warning::ci/test-plan.json: mem-test.{key} has an unknown value {x!r}")
        return [a for a in allowed if a.upper() in [w.upper() for w in wanted]]

    return names("exes", EXES), names("scenarios", SCENARIOS), names("quick", SCENARIOS), read_options(entry)


def legs(exes, scenarios, quick, options):
    out = []

    def leg(name, versions, scens, scale="1"):
        if versions and scens:
            item = {"leg": name, "versions": ",".join(versions), "scenarios": ",".join(scens), "time_scale": scale, "size_scale": scale}
            item.update(options)
            out.append(item)

    if exes:
        leg("quick", exes[:1], [s for s in quick if s != "D"], "0.05")
    for exe in exes:
        wanted = [s for s in scenarios if s in ("A", "F") or (s == "G" and exe == "branch")]
        leg(f"churn-{exe}", [exe], wanted)
        if exe == "branch":
            leg("hook-branch", [exe], [s for s in scenarios if s == "H"])
    for exe in sorted(exes, key=lambda e: e != "branch"):
        leg(f"robust-{exe}", [exe], [s for s in scenarios if s in ("O", "K", "M", "S", "Q")])
    update = []
    if "D" in scenarios:
        if all(v in exes for v in RELEASED):
            update.append("D")
        else:
            print("::warning::scenario D needs both 0.9.1 and 0.10.0 in mem-test.exes; skipped")
    if "G" in scenarios and "0.10.0" in exes:
        update.append("G")
    leg("update", [v for v in ("0.9.1", "0.10.0") if v in exes], update)
    for exe in exes:
        leg(f"static-{exe}", [exe], [s for s in scenarios if s in ("B", "C", "E")])
    return out


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "ci/test-plan.json"
    event = os.environ.get("EVENT_NAME", "push")
    exes, scenarios, quick, options = read_plan(path, event)
    plan = legs(exes, scenarios, quick, options)
    if plan:
        print(f"hook {options['save_delay_var']}={options['save_delay_ms']}, fail on criteria: {options['fail_on_criteria']}")
    for item in plan:
        print(f"leg {item['leg']}: exe {item['versions']}, scenarios {item['scenarios']}, scale {item['time_scale']}")
    if not plan:
        print("::notice::mem-test: nothing to run (see ci/test-plan.json)")
    versions = {v for item in plan for v in item["versions"].split(",")}
    outputs = {
        # A matrix must not be empty, even for a job that is skipped.
        "matrix": json.dumps({"include": plan or [dict({"leg": "none", "versions": "", "scenarios": "", "time_scale": "1", "size_scale": "1"}, **options)]}),
        "any": "true" if plan else "false",
        "branch": "true" if "branch" in versions else "false",
        "released": "true" if versions & set(RELEASED) else "false",
    }
    target = os.environ.get("GITHUB_OUTPUT")
    if target:
        with open(target, "a", encoding="utf-8") as f:
            for key, value in outputs.items():
                f.write(f"{key}={value}\n")
    else:
        print(json.dumps(outputs, indent=2))


if __name__ == "__main__":
    main()
