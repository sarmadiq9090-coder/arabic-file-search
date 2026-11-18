# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-18. This build: bahth 1.1.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 643 | 613 | -5% | 2091 / 2091 |
| `notepad` | 654 | 632 | -3% | 48 / 48 |
| `احمد` | 645 | 755 | +17% | 0 / 0 |
| `تقرير مالي` | 635 | 607 | -4% | 1 / 1 |
| `مشكلة` | 637 | 610 | -4% | 2 / 2 |
| `ext:dll` | 697 | 683 | -2% | 111548 / 111548 |
| `system32 config` | 634 | 896 | +41% | 0 / 326 |
| **all** | **4,545** | **4,796** | **+6%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 32 | 34 | +6% |
| Built | 156 | 159 | +2% |
| Layout | 156 | 158 | +1% |
| FirstTick | 316 | 310 | -2% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=13 font=13 settings=13 window=50 list=90 controls=116 tray=122 events=124 layout=156 built=156 loop=156 first-tick=317`
- this build: `single-instance=13 init=15 font=15 settings=17 window=50 list=85 controls=106 tray=112 events=135 layout=157 built=157 loop=157 first-tick=308`

Quick search popup (this build only): opened in a median of 9 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,552 size 480x116
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 142, mean 44
- [ar] 'المفكرة': height 253 (empty 142), sd 34.9
- [ar] 'كروم': height 219 (empty 142), sd 35.5
- [ar] 'واي فاي': height 285 (empty 142), sd 33.7
- [ar] '250*12': height 219 (empty 142), sd 35.6
- [ar] '5 kg to lb': height 219 (empty 142), sd 36.6
- [ar] 'هجري': height 285 (empty 142), sd 33.9
- [ar] 'يوتيوب قرآن': height 319 (empty 142), sd 33.7
- [ar] 'اطفاء': height 285 (empty 142), sd 33.6
- [ar] 'مشكلة': height 531 (empty 142), sd 35.0
- [ar] 'qzxwvkjh': height 319 (empty 142), sd 34.4
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,552 size 480x116
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 142, mean 45
- [en] 'notepad': height 365 (empty 142), sd 35.9
- [en] 'paint': height 485 (empty 142), sd 36.3
- [en] 'واي فاي': height 285 (empty 142), sd 35.4
- [en] '250*12': height 219 (empty 142), sd 35.9
- [en] '5 kg to lb': height 219 (empty 142), sd 37.0
- [en] 'هجري': height 285 (empty 142), sd 36.0
- [en] 'يوتيوب قرآن': height 319 (empty 142), sd 36.8
- [en] 'اطفاء': height 285 (empty 142), sd 34.7
- [en] 'مشكلة': height 531 (empty 142), sd 36.2
- [en] 'qzxwvkjh': height 319 (empty 142), sd 37.2
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 142, mean 193
- [ar-light] 'calculator': height 459 (empty 142), sd 71.1
- [ar-light] '250*12': height 193 (empty 142), sd 84.8
- [ar-light] 'مشكلة': height 505 (empty 142), sd 63.7
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 336 ms

**Failed:**

- 'system32 config' finds 0 with 0.6.1 but 326 now
- the command line is slower than 0.6.1: 4,796 ms against 4,545 ms
