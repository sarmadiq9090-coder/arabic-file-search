# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 9V74 80-Core Processor, 2026-11-06. This build: bahth 0.9.1. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 612 | 615 | +0% | 2089 / 2089 |
| `notepad` | 623 | 616 | -1% | 48 / 48 |
| `احمد` | 615 | 609 | -1% | 0 / 0 |
| `تقرير مالي` | 612 | 611 | +0% | 1 / 1 |
| `مشكلة` | 613 | 609 | -1% | 2 / 2 |
| `ext:dll` | 670 | 669 | +0% | 111585 / 111585 |
| `system32 config` | 613 | 610 | +0% | 0 / 0 |
| **all** | **4,358** | **4,338** | **+0%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 29 | 33 | +14% |
| Built | 120 | 119 | -1% |
| Layout | 118 | 119 | +1% |
| FirstTick | 287 | 285 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=9 init=10 font=10 settings=10 window=32 list=67 controls=84 tray=91 events=92 layout=114 built=114 loop=114 first-tick=286`
- this build: `single-instance=8 init=10 font=10 settings=10 window=36 list=68 controls=84 tray=90 events=107 layout=123 built=123 loop=123 first-tick=285`

Quick search popup (this build only): opened in a median of 6 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.48
- [ar] popup from the box at 392,546 size 480x122
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 122, mean 47
- [ar] 'المفكرة': height 233 (empty 122), sd 33.1
- [ar] 'كروم': height 199 (empty 122), sd 34.3
- [ar] 'واي فاي': height 265 (empty 122), sd 31.9
- [ar] '250*12': height 199 (empty 122), sd 34.2
- [ar] '5 kg to lb': height 199 (empty 122), sd 35.4
- [ar] 'هجري': height 265 (empty 122), sd 32.3
- [ar] 'يوتيوب قرآن': height 299 (empty 122), sd 32.1
- [ar] 'اطفاء': height 265 (empty 122), sd 31.8
- [ar] 'مشكلة': height 511 (empty 122), sd 34.3
- [ar] 'qzxwvkjh': height 299 (empty 122), sd 32.7
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.55
- [en] popup from the box at 392,546 size 480x122
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 122, mean 47
- [en] 'notepad': height 345 (empty 122), sd 35.4
- [en] 'paint': height 465 (empty 122), sd 35.4
- [en] 'واي فاي': height 265 (empty 122), sd 34.2
- [en] '250*12': height 199 (empty 122), sd 35.6
- [en] '5 kg to lb': height 199 (empty 122), sd 36.7
- [en] 'هجري': height 265 (empty 122), sd 34.7
- [en] 'يوتيوب قرآن': height 299 (empty 122), sd 35.2
- [en] 'اطفاء': height 265 (empty 122), sd 33.5
- [en] 'مشكلة': height 511 (empty 122), sd 35.4
- [en] 'qzxwvkjh': height 299 (empty 122), sd 35.5
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.44
- [ar-light] popup empty: height 122, mean 192
- [ar-light] 'calculator': height 465 (empty 122), sd 65.1
- [ar-light] '250*12': height 199 (empty 122), sd 72.3
- [ar-light] 'مشكلة': height 511 (empty 122), sd 64.6
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 95 ms

All checks passed.
