# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-01. This build: bahth 0.7.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 654 | 648 | -1% | 2089 / 2089 |
| `notepad` | 672 | 664 | -1% | 48 / 48 |
| `احمد` | 658 | 647 | -2% | 0 / 0 |
| `تقرير مالي` | 651 | 643 | -1% | 1 / 1 |
| `مشكلة` | 640 | 640 | +0% | 2 / 2 |
| `ext:dll` | 733 | 707 | -4% | 111790 / 111790 |
| `system32 config` | 655 | 649 | -1% | 0 / 0 |
| **all** | **4,663** | **4,598** | **-1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 34 | 34 | +1% |
| Built | 157 | 166 | +6% |
| Layout | 156 | 165 | +6% |
| FirstTick | 318 | 319 | +0% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=14 font=14 settings=14 window=48 list=88 controls=112 tray=118 events=120 layout=151 built=151 loop=151 first-tick=313`
- this build: `single-instance=10 init=14 font=14 settings=14 window=51 list=92 controls=115 tray=123 events=125 layout=164 built=164 loop=164 first-tick=316`

Quick search popup (this build only): opened in a median of 12 ms over 8 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,572 size 480x96
- [ar] popup empty: height 96, mean 40
- [ar] 'المفكرة': height 207 (empty 96), sd 33.4
- [ar] 'كروم': height 173 (empty 96), sd 34.3
- [ar] 'واي فاي': height 239 (empty 96), sd 33.1
- [ar] '250*12': height 173 (empty 96), sd 34.4
- [ar] '5 kg to lb': height 173 (empty 96), sd 35.7
- [ar] 'هجري': height 239 (empty 96), sd 33.5
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 34.4
- [ar] 'اطفاء': height 239 (empty 96), sd 32.9
- [ar] 'مشكلة': height 339 (empty 96), sd 34.1
- [ar] 'qzxwvkjh': height 173 (empty 96), sd 36.1
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,572 size 480x96
- [en] popup empty: height 96, mean 41
- [en] 'notepad': height 319 (empty 96), sd 35.5
- [en] 'paint': height 439 (empty 96), sd 35.6
- [en] 'واي فاي': height 239 (empty 96), sd 35.9
- [en] '250*12': height 173 (empty 96), sd 35.3
- [en] '5 kg to lb': height 173 (empty 96), sd 36.5
- [en] 'هجري': height 239 (empty 96), sd 36.3
- [en] 'يوتيوب قرآن': height 207 (empty 96), sd 36.7
- [en] 'اطفاء': height 239 (empty 96), sd 35.1
- [en] 'مشكلة': height 339 (empty 96), sd 35.4
- [en] 'qzxwvkjh': height 207 (empty 96), sd 37.3
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 96, mean 181
- [ar-light] 'calculator': height 439 (empty 96), sd 64.7
- [ar-light] '250*12': height 173 (empty 96), sd 73.8
- [ar-light] 'مشكلة': height 339 (empty 96), sd 66.4
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 144 apps, 188 settings pages in 111 ms

All checks passed.
