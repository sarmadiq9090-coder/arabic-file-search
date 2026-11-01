# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-01. This build: bahth 0.7.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 626 | 624 | +0% | 2089 / 2089 |
| `notepad` | 634 | 626 | -1% | 48 / 48 |
| `احمد` | 626 | 623 | -1% | 0 / 0 |
| `تقرير مالي` | 636 | 632 | -1% | 1 / 1 |
| `مشكلة` | 629 | 625 | -1% | 2 / 2 |
| `ext:dll` | 679 | 674 | -1% | 111790 / 111790 |
| `system32 config` | 630 | 622 | -1% | 0 / 0 |
| **all** | **4,460** | **4,426** | **-1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 31 | 33 | +5% |
| Built | 147 | 155 | +5% |
| Layout | 147 | 155 | +5% |
| FirstTick | 312 | 312 | +0% |

Raw phases (last run of each):

- 0.6.1: `single-instance=11 init=13 font=13 settings=13 window=46 list=84 controls=107 tray=114 events=115 layout=146 built=146 loop=146 first-tick=308`
- this build: `single-instance=10 init=12 font=12 settings=12 window=48 list=86 controls=112 tray=118 events=119 layout=157 built=158 loop=158 first-tick=312`

Quick search popup (this build only): opened in a median of 13 ms over 6 openings (`quick search shown in` in log.txt).

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
- [ar] 'هجري': height 239 (empty 96), sd 33.3
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 34.3
- [ar] 'اطفاء': height 239 (empty 96), sd 32.8
- [ar] 'مشكلة': height 339 (empty 96), sd 34.1
- [ar] 'qzxwvkjh': height 207 (empty 96), sd 35.0
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 0.00
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
- catalog: launcher catalog: 144 apps, 188 settings pages in 115 ms

**Failed:**

- the floating box does not fade in on hover (en)
- a click on the floating box did not open the popup (en)
- the popup did not open with Ctrl+Alt+F (en)
