# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz, 2026-11-01. This build: bahth 0.7.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 1,004 | 1,005 | +0% | 2091 / 2091 |
| `notepad` | 1,004 | 1,005 | +0% | 48 / 48 |
| `احمد` | 1,005 | 1,004 | +0% | 0 / 0 |
| `تقرير مالي` | 1,005 | 1,004 | +0% | 1 / 1 |
| `مشكلة` | 1,005 | 1,005 | +0% | 2 / 2 |
| `ext:dll` | 1,005 | 1,005 | +0% | 111790 / 111790 |
| `system32 config` | 1,004 | 1,004 | +0% | 0 / 0 |
| **all** | **7,031** | **7,032** | **+0%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 31 | 33 | +7% |
| Built | 148 | 148 | +0% |
| Layout | 148 | 148 | +0% |
| FirstTick | 309 | 313 | +1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=12 font=12 settings=12 window=44 list=80 controls=99 tray=107 events=108 layout=135 built=135 loop=135 first-tick=302`
- this build: `single-instance=10 init=12 font=12 settings=14 window=46 list=88 controls=109 tray=115 events=116 layout=146 built=147 loop=147 first-tick=311`

Quick search popup (this build only): opened in a median of 11 ms over 5 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.27
- [ar] popup from the box at 392,572 size 480x96
- [ar] popup empty: height 96, mean 51
- [ar] 'المفكرة': height 207 (empty 96), sd 38.6
- [ar] 'كروم': height 173 (empty 96), sd 40.0
- [ar] 'واي فاي': height 239 (empty 96), sd 37.7
- [ar] '250*12': height 173 (empty 96), sd 39.9
- [ar] '5 kg to lb': height 173 (empty 96), sd 40.8
- [ar] 'هجري': height 239 (empty 96), sd 38.0
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 39.3
- [ar] 'اطفاء': height 239 (empty 96), sd 37.6
- [ar] 'مشكلة': height 339 (empty 96), sd 37.0
- [ar] 'qzxwvkjh': height 207 (empty 96), sd 39.8
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.26
- [en] popup from the box at 392,572 size 480x96
- [en] popup empty: height 96, mean 51
- [en] 'notepad': height 319 (empty 96), sd 37.8
- [en] 'paint': height 439 (empty 96), sd 36.6
- [en] 'واي فاي': height 239 (empty 96), sd 39.0
- [en] '250*12': height 173 (empty 96), sd 40.1
- [en] '5 kg to lb': height 173 (empty 96), sd 41.0
- [en] 'هجري': height 239 (empty 96), sd 39.2
- [en] 'يوتيوب قرآن': height 207 (empty 96), sd 40.5
- [en] 'اطفاء': height 239 (empty 96), sd 38.4
- [en] 'مشكلة': height 339 (empty 96), sd 37.6
- [en] 'qzxwvkjh': height 207 (empty 96), sd 40.9
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 7.31
- [ar-light] popup empty: height 96, mean 173
- [ar-light] 'calculator': height 439 (empty 96), sd 58.7
- [ar-light] '250*12': height 173 (empty 96), sd 69.6
- [ar-light] 'مشكلة': height 339 (empty 96), sd 60.9
- catalog: launcher catalog: 144 apps, 188 settings pages in 102 ms

All checks passed.
