# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 9V74 80-Core Processor, 2026-11-18. This build: bahth 1.1.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 613 | 595 | -3% | 2091 / 2091 |
| `notepad` | 614 | 595 | -3% | 48 / 48 |
| `احمد` | 615 | 704 | +14% | 0 / 0 |
| `تقرير مالي` | 615 | 595 | -3% | 1 / 1 |
| `مشكلة` | 610 | 589 | -3% | 2 / 2 |
| `ext:dll` | 662 | 651 | -2% | 111549 / 111549 |
| `system32 config` | 610 | 801 | +31% | 0 / 326 |
| **all** | **4,339** | **4,531** | **+4%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 31 | 30 | -2% |
| Built | 115 | 118 | +3% |
| Layout | 114 | 118 | +4% |
| FirstTick | 285 | 281 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=9 init=10 font=10 settings=10 window=40 list=76 controls=96 tray=102 events=103 layout=128 built=128 loop=128 first-tick=296`
- this build: `single-instance=12 init=14 font=14 settings=16 window=41 list=71 controls=89 tray=95 events=114 layout=129 built=129 loop=129 first-tick=291`

Quick search popup (this build only): opened in a median of 6 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.48
- [ar] popup from the box at 392,552 size 480x116
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 142, mean 46
- [ar] 'المفكرة': height 253 (empty 142), sd 36.7
- [ar] 'كروم': height 219 (empty 142), sd 37.3
- [ar] 'واي فاي': height 285 (empty 142), sd 36.2
- [ar] '250*12': height 219 (empty 142), sd 37.3
- [ar] '5 kg to lb': height 219 (empty 142), sd 38.3
- [ar] 'هجري': height 285 (empty 142), sd 36.5
- [ar] 'يوتيوب قرآن': height 319 (empty 142), sd 35.6
- [ar] 'اطفاء': height 285 (empty 142), sd 36.1
- [ar] 'مشكلة': height 531 (empty 142), sd 34.8
- [ar] 'qzxwvkjh': height 319 (empty 142), sd 33.4
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.55
- [en] popup from the box at 392,552 size 480x116
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 142, mean 47
- [en] 'notepad': height 365 (empty 142), sd 36.5
- [en] 'paint': height 485 (empty 142), sd 36.1
- [en] 'واي فاي': height 285 (empty 142), sd 35.6
- [en] '250*12': height 219 (empty 142), sd 35.9
- [en] '5 kg to lb': height 219 (empty 142), sd 37.0
- [en] 'هجري': height 285 (empty 142), sd 36.2
- [en] 'يوتيوب قرآن': height 319 (empty 142), sd 36.2
- [en] 'اطفاء': height 285 (empty 142), sd 34.9
- [en] 'مشكلة': height 531 (empty 142), sd 36.2
- [en] 'qzxwvkjh': height 319 (empty 142), sd 36.6
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.44
- [ar-light] popup empty: height 116, mean 190
- [ar-light] 'calculator': height 485 (empty 116), sd 69.7
- [ar-light] '250*12': height 219 (empty 116), sd 80.4
- [ar-light] 'مشكلة': height 531 (empty 116), sd 70.5
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 88 ms

**Failed:**

- 'system32 config' finds 0 with 0.6.1 but 326 now
