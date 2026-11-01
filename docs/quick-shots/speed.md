# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-01. This build: bahth 0.7.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 631 | 636 | +1% | 2089 / 2089 |
| `notepad` | 635 | 634 | +0% | 48 / 48 |
| `احمد` | 635 | 631 | -1% | 0 / 0 |
| `تقرير مالي` | 629 | 627 | +0% | 1 / 1 |
| `مشكلة` | 628 | 623 | -1% | 2 / 2 |
| `ext:dll` | 696 | 685 | -2% | 111794 / 111794 |
| `system32 config` | 627 | 631 | +1% | 0 / 0 |
| **all** | **4,481** | **4,467** | **+0%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 31 | 31 | +0% |
| Built | 156 | 161 | +3% |
| Layout | 156 | 161 | +3% |
| FirstTick | 316 | 316 | +0% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=12 font=12 settings=12 window=48 list=88 controls=111 tray=118 events=120 layout=150 built=151 loop=151 first-tick=312`
- this build: `single-instance=10 init=12 font=12 settings=12 window=50 list=92 controls=114 tray=122 events=123 layout=160 built=160 loop=160 first-tick=316`

Quick search popup (this build only): opened in a median of 13 ms over 8 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,572 size 480x96
- [ar] popup empty: height 96, mean 41
- [ar] 'المفكرة': height 207 (empty 96), sd 33.3
- [ar] 'كروم': height 173 (empty 96), sd 34.3
- [ar] 'واي فاي': height 239 (empty 96), sd 33.0
- [ar] '250*12': height 173 (empty 96), sd 34.4
- [ar] '5 kg to lb': height 173 (empty 96), sd 35.6
- [ar] 'هجري': height 239 (empty 96), sd 33.4
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 34.3
- [ar] 'اطفاء': height 239 (empty 96), sd 32.8
- [ar] 'مشكلة': height 339 (empty 96), sd 34.1
- [ar] 'qzxwvkjh': height 207 (empty 96), sd 35.0
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,572 size 480x96
- [en] popup empty: height 96, mean 41
- [en] 'notepad': height 319 (empty 96), sd 35.5
- [en] 'paint': height 439 (empty 96), sd 35.6
- [en] 'واي فاي': height 239 (empty 96), sd 35.7
- [en] '250*12': height 173 (empty 96), sd 35.2
- [en] '5 kg to lb': height 173 (empty 96), sd 36.5
- [en] 'هجري': height 239 (empty 96), sd 36.1
- [en] 'يوتيوب قرآن': height 207 (empty 96), sd 36.6
- [en] 'اطفاء': height 239 (empty 96), sd 34.9
- [en] 'مشكلة': height 339 (empty 96), sd 35.5
- [en] 'qzxwvkjh': height 207 (empty 96), sd 37.2
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 96, mean 181
- [ar-light] 'calculator': height 439 (empty 96), sd 64.6
- [ar-light] '250*12': height 173 (empty 96), sd 74.0
- [ar-light] 'مشكلة': height 339 (empty 96), sd 66.4
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 144 apps, 188 settings pages in 111 ms

All checks passed.
