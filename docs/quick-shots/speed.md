# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-02. This build: bahth 0.8.2. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 639 | 635 | -1% | 2089 / 2089 |
| `notepad` | 638 | 647 | +1% | 48 / 48 |
| `احمد` | 634 | 644 | +2% | 0 / 0 |
| `تقرير مالي` | 638 | 648 | +1% | 1 / 1 |
| `مشكلة` | 632 | 639 | +1% | 2 / 2 |
| `ext:dll` | 689 | 701 | +2% | 111794 / 111794 |
| `system32 config` | 633 | 638 | +1% | 0 / 0 |
| **all** | **4,503** | **4,550** | **+1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 32 | 33 | +4% |
| Built | 158 | 159 | +1% |
| Layout | 156 | 157 | +1% |
| FirstTick | 319 | 312 | -2% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=12 font=12 settings=12 window=50 list=92 controls=118 tray=125 events=127 layout=156 built=158 loop=158 first-tick=319`
- this build: `single-instance=11 init=14 font=14 settings=14 window=48 list=88 controls=110 tray=116 events=138 layout=159 built=159 loop=159 first-tick=312`

Quick search popup (this build only): opened in a median of 9 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,572 size 480x96
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 96, mean 41
- [ar] 'المفكرة': height 207 (empty 96), sd 32.7
- [ar] 'كروم': height 173 (empty 96), sd 33.6
- [ar] 'واي فاي': height 239 (empty 96), sd 33.5
- [ar] '250*12': height 173 (empty 96), sd 33.5
- [ar] '5 kg to lb': height 465 (empty 96), sd 34.1
- [ar] 'هجري': height 273 (empty 96), sd 33.0
- [ar] 'يوتيوب قرآن': height 273 (empty 96), sd 33.0
- [ar] 'اطفاء': height 239 (empty 96), sd 33.4
- [ar] 'مشكلة': height 445 (empty 96), sd 35.1
- [ar] 'qzxwvkjh': height 273 (empty 96), sd 33.5
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,572 size 480x96
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 96, mean 41
- [en] 'notepad': height 319 (empty 96), sd 35.5
- [en] 'paint': height 439 (empty 96), sd 35.7
- [en] 'واي فاي': height 239 (empty 96), sd 35.9
- [en] '250*12': height 173 (empty 96), sd 35.2
- [en] '5 kg to lb': height 173 (empty 96), sd 36.4
- [en] 'هجري': height 239 (empty 96), sd 36.4
- [en] 'يوتيوب قرآن': height 273 (empty 96), sd 36.1
- [en] 'اطفاء': height 239 (empty 96), sd 35.1
- [en] 'مشكلة': height 445 (empty 96), sd 36.4
- [en] 'qzxwvkjh': height 273 (empty 96), sd 36.6
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 96, mean 181
- [ar-light] 'calculator': height 439 (empty 96), sd 64.6
- [ar-light] '250*12': height 173 (empty 96), sd 74.0
- [ar-light] 'مشكلة': height 445 (empty 96), sd 63.3
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 144 apps, 188 settings pages in 120 ms

All checks passed.
