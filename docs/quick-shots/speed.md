# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 7763 64-Core Processor, 2026-11-18. This build: bahth 1.1.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 643 | 621 | -3% | 2091 / 2091 |
| `notepad` | 647 | 621 | -4% | 48 / 48 |
| `احمد` | 649 | 752 | +16% | 0 / 0 |
| `تقرير مالي` | 643 | 616 | -4% | 1 / 1 |
| `مشكلة` | 640 | 619 | -3% | 2 / 2 |
| `ext:dll` | 689 | 686 | +0% | 111548 / 111548 |
| `system32 config` | 647 | 893 | +38% | 0 / 326 |
| **all** | **4,556** | **4,808** | **+6%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 33 | 31 | -5% |
| Built | 152 | 157 | +3% |
| Layout | 150 | 156 | +4% |
| FirstTick | 315 | 307 | -3% |

Raw phases (last run of each):

- 0.6.1: `single-instance=10 init=12 font=12 settings=12 window=48 list=86 controls=112 tray=119 events=120 layout=150 built=152 loop=152 first-tick=314`
- this build: `single-instance=14 init=16 font=17 settings=17 window=48 list=84 controls=104 tray=111 events=134 layout=156 built=156 loop=156 first-tick=305`

Quick search popup (this build only): opened in a median of 9 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.51
- [ar] popup from the box at 392,552 size 480x116
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 142, mean 44
- [ar] 'المفكرة': height 253 (empty 142), sd 35.0
- [ar] 'كروم': height 219 (empty 142), sd 35.7
- [ar] 'واي فاي': height 285 (empty 142), sd 33.8
- [ar] '250*12': height 219 (empty 142), sd 35.7
- [ar] '5 kg to lb': height 219 (empty 142), sd 36.7
- [ar] 'هجري': height 285 (empty 142), sd 34.1
- [ar] 'يوتيوب قرآن': height 319 (empty 142), sd 33.8
- [ar] 'اطفاء': height 285 (empty 142), sd 33.7
- [ar] 'مشكلة': height 531 (empty 142), sd 35.0
- [ar] 'qzxwvkjh': height 319 (empty 142), sd 34.4
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.58
- [en] popup from the box at 392,552 size 480x116
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 142, mean 45
- [en] 'notepad': height 365 (empty 142), sd 36.0
- [en] 'paint': height 485 (empty 142), sd 36.4
- [en] 'واي فاي': height 285 (empty 142), sd 35.5
- [en] '250*12': height 219 (empty 142), sd 36.1
- [en] '5 kg to lb': height 219 (empty 142), sd 37.2
- [en] 'هجري': height 285 (empty 142), sd 36.1
- [en] 'يوتيوب قرآن': height 319 (empty 142), sd 36.9
- [en] 'اطفاء': height 285 (empty 142), sd 34.8
- [en] 'مشكلة': height 531 (empty 142), sd 36.3
- [en] 'qzxwvkjh': height 319 (empty 142), sd 37.3
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.33
- [ar-light] popup empty: height 116, mean 188
- [ar-light] 'calculator': height 459 (empty 116), sd 71.0
- [ar-light] '250*12': height 193 (empty 116), sd 84.8
- [ar-light] 'مشكلة': height 531 (empty 116), sd 65.8
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, smartscreen, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 244 ms

**Failed:**

- 'system32 config' finds 0 with 0.6.1 but 326 now
- the command line is slower than 0.6.1: 4,808 ms against 4,556 ms
