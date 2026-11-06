# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-06. This build: bahth 0.9.1. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 639 | 635 | -1% | 2091 / 2091 |
| `notepad` | 648 | 651 | +0% | 48 / 48 |
| `احمد` | 642 | 643 | +0% | 0 / 0 |
| `تقرير مالي` | 646 | 643 | -1% | 1 / 1 |
| `مشكلة` | 655 | 652 | -1% | 2 / 2 |
| `ext:dll` | 712 | 706 | -1% | 111549 / 111549 |
| `system32 config` | 649 | 652 | +1% | 0 / 0 |
| **all** | **4,590** | **4,581** | **+0%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 31 | 37 | +20% |
| Built | 147 | 154 | +5% |
| Layout | 147 | 153 | +4% |
| FirstTick | 311 | 309 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=11 init=13 font=13 settings=13 window=48 list=85 controls=108 tray=114 events=115 layout=145 built=146 loop=146 first-tick=308`
- this build: `single-instance=10 init=12 font=12 settings=12 window=48 list=86 controls=110 tray=115 events=136 layout=156 built=158 loop=158 first-tick=311`

Quick search popup (this build only): opened in a median of 9 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,546 size 480x122
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 122, mean 44
- [ar] 'المفكرة': height 233 (empty 122), sd 32.4
- [ar] 'كروم': height 199 (empty 122), sd 33.3
- [ar] 'واي فاي': height 265 (empty 122), sd 32.2
- [ar] '250*12': height 199 (empty 122), sd 33.4
- [ar] '5 kg to lb': height 199 (empty 122), sd 34.4
- [ar] 'هجري': height 265 (empty 122), sd 32.7
- [ar] 'يوتيوب قرآن': height 299 (empty 122), sd 32.9
- [ar] 'اطفاء': height 265 (empty 122), sd 32.2
- [ar] 'مشكلة': height 511 (empty 122), sd 35.1
- [ar] 'qzxwvkjh': height 299 (empty 122), sd 33.4
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,546 size 480x122
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 122, mean 44
- [en] 'notepad': height 345 (empty 122), sd 35.3
- [en] 'paint': height 531 (empty 122), sd 31.3
- [en] 'واي فاي': height 265 (empty 122), sd 37.2
- [en] '250*12': height 199 (empty 122), sd 34.7
- [en] '5 kg to lb': height 199 (empty 122), sd 35.7
- [en] 'هجري': height 265 (empty 122), sd 35.0
- [en] 'يوتيوب قرآن': height 299 (empty 122), sd 35.8
- [en] 'اطفاء': height 265 (empty 122), sd 33.8
- [en] 'مشكلة': height 511 (empty 122), sd 36.1
- [en] 'qzxwvkjh': height 299 (empty 122), sd 36.1
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 122, mean 188
- [ar-light] 'calculator': height 465 (empty 122), sd 64.2
- [ar-light] '250*12': height 199 (empty 122), sd 72.2
- [ar-light] 'مشكلة': height 511 (empty 122), sd 62.3
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 111 ms

All checks passed.
