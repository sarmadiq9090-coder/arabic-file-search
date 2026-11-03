# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 9V74 80-Core Processor, 2026-11-02. This build: bahth 0.9.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 701 | 699 | +0% | 2089 / 2089 |
| `notepad` | 690 | 695 | +1% | 48 / 48 |
| `احمد` | 723 | 698 | -3% | 0 / 0 |
| `تقرير مالي` | 695 | 689 | -1% | 1 / 1 |
| `مشكلة` | 694 | 696 | +0% | 2 / 2 |
| `ext:dll` | 753 | 757 | +1% | 111790 / 111790 |
| `system32 config` | 695 | 699 | +0% | 0 / 0 |
| **all** | **4,950** | **4,932** | **+0%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 32 | 33 | +1% |
| Built | 141 | 148 | +5% |
| Layout | 141 | 148 | +5% |
| FirstTick | 304 | 302 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=13 font=13 settings=13 window=41 list=81 controls=103 tray=109 events=111 layout=140 built=140 loop=140 first-tick=304`
- this build: `single-instance=10 init=12 font=14 settings=14 window=48 list=84 controls=102 tray=109 events=131 layout=154 built=154 loop=154 first-tick=305`

Quick search popup (this build only): opened in a median of 7 ms over 51 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,572 size 480x96
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 96, mean 40
- [ar] 'المفكرة': height 207 (empty 96), sd 32.6
- [ar] 'كروم': height 173 (empty 96), sd 33.7
- [ar] 'واي فاي': height 239 (empty 96), sd 33.5
- [ar] '250*12': height 173 (empty 96), sd 33.6
- [ar] '5 kg to lb': height 173 (empty 96), sd 34.9
- [ar] 'هجري': height 239 (empty 96), sd 34.0
- [ar] 'يوتيوب قرآن': height 273 (empty 96), sd 33.1
- [ar] 'اطفاء': height 239 (empty 96), sd 33.4
- [ar] 'مشكلة': height 445 (empty 96), sd 35.1
- [ar] 'qzxwvkjh': height 273 (empty 96), sd 33.6
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,572 size 480x96
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 96, mean 41
- [en] 'notepad': height 319 (empty 96), sd 35.5
- [en] 'paint': height 439 (empty 96), sd 35.7
- [en] 'واي فاي': height 239 (empty 96), sd 35.9
- [en] '250*12': height 173 (empty 96), sd 35.2
- [en] '5 kg to lb': height 173 (empty 96), sd 36.5
- [en] 'هجري': height 239 (empty 96), sd 36.4
- [en] 'يوتيوب قرآن': height 273 (empty 96), sd 36.1
- [en] 'اطفاء': height 239 (empty 96), sd 35.2
- [en] 'مشكلة': height 445 (empty 96), sd 36.4
- [en] 'qzxwvkjh': height 273 (empty 96), sd 36.6
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 96, mean 181
- [ar-light] 'calculator': height 439 (empty 96), sd 64.6
- [ar-light] '250*12': height 96 (empty 96), sd 83.7
- [ar-light] 'مشكلة': height 307 (empty 96), sd 67.2
- [ar-light] first attempt failed (no results for '250*12' (ar-light): the popup did not grow); running the session again
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 96, mean 181
- [ar-light] 'calculator': height 439 (empty 96), sd 64.6
- [ar-light] '250*12': height 173 (empty 96), sd 73.9
- [ar-light] 'مشكلة': height 445 (empty 96), sd 63.3
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, smartscreen, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 144 apps, 188 settings pages in 109 ms

All checks passed.
