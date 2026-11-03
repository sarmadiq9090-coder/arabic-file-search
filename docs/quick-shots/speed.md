# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-03. This build: bahth 0.9.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 655 | 666 | +2% | 2089 / 2089 |
| `notepad` | 655 | 659 | +1% | 48 / 48 |
| `احمد` | 651 | 660 | +1% | 0 / 0 |
| `تقرير مالي` | 648 | 650 | +0% | 1 / 1 |
| `مشكلة` | 654 | 657 | +0% | 2 / 2 |
| `ext:dll` | 703 | 704 | +0% | 111585 / 111585 |
| `system32 config` | 649 | 647 | +0% | 0 / 0 |
| **all** | **4,614** | **4,643** | **+1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 31 | 31 | +0% |
| Built | 152 | 156 | +3% |
| Layout | 151 | 156 | +3% |
| FirstTick | 314 | 310 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=12 font=12 settings=12 window=48 list=88 controls=112 tray=118 events=120 layout=150 built=150 loop=150 first-tick=313`
- this build: `single-instance=10 init=12 font=12 settings=12 window=47 list=85 controls=108 tray=114 events=134 layout=155 built=156 loop=156 first-tick=309`

Quick search popup (this build only): opened in a median of 9 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,572 size 480x96
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 96, mean 41
- [ar] 'المفكرة': height 207 (empty 96), sd 33.3
- [ar] 'كروم': height 173 (empty 96), sd 34.2
- [ar] 'واي فاي': height 239 (empty 96), sd 32.9
- [ar] '250*12': height 173 (empty 96), sd 34.2
- [ar] '5 kg to lb': height 173 (empty 96), sd 35.4
- [ar] 'هجري': height 239 (empty 96), sd 33.4
- [ar] 'يوتيوب قرآن': height 273 (empty 96), sd 33.6
- [ar] 'اطفاء': height 239 (empty 96), sd 32.8
- [ar] 'مشكلة': height 485 (empty 96), sd 35.5
- [ar] 'qzxwvkjh': height 273 (empty 96), sd 34.1
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,572 size 480x96
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 96, mean 41
- [en] 'notepad': height 319 (empty 96), sd 35.4
- [en] 'paint': height 439 (empty 96), sd 35.6
- [en] 'واي فاي': height 239 (empty 96), sd 35.9
- [en] '250*12': height 173 (empty 96), sd 35.0
- [en] '5 kg to lb': height 173 (empty 96), sd 36.4
- [en] 'هجري': height 239 (empty 96), sd 36.3
- [en] 'يوتيوب قرآن': height 273 (empty 96), sd 36.0
- [en] 'اطفاء': height 239 (empty 96), sd 35.2
- [en] 'مشكلة': height 485 (empty 96), sd 36.2
- [en] 'qzxwvkjh': height 273 (empty 96), sd 36.5
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 96, mean 181
- [ar-light] 'calculator': height 439 (empty 96), sd 64.6
- [ar-light] '250*12': height 173 (empty 96), sd 74.1
- [ar-light] 'مشكلة': height 485 (empty 96), sd 63.1
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 117 ms

All checks passed.
