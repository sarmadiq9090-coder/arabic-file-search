# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 9V74 80-Core Processor, 2026-11-01. This build: bahth 0.7.0. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 585 | 582 | -1% | 2091 / 2091 |
| `notepad` | 582 | 578 | -1% | 48 / 48 |
| `احمد` | 583 | 578 | -1% | 0 / 0 |
| `تقرير مالي` | 581 | 579 | +0% | 1 / 1 |
| `مشكلة` | 581 | 575 | -1% | 2 / 2 |
| `ext:dll` | 631 | 627 | -1% | 111791 / 111791 |
| `system32 config` | 582 | 579 | -1% | 0 / 0 |
| **all** | **4,125** | **4,098** | **-1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 25 | 28 | +15% |
| Built | 104 | 108 | +4% |
| Layout | 104 | 108 | +4% |
| FirstTick | 276 | 277 | +0% |

Raw phases (last run of each):

- 0.6.1: `single-instance=8 init=10 font=11 settings=11 window=30 list=57 controls=75 tray=82 events=82 layout=104 built=104 loop=104 first-tick=276`
- this build: `single-instance=8 init=11 font=11 settings=11 window=30 list=58 controls=74 tray=81 events=82 layout=108 built=108 loop=108 first-tick=276`

Quick search popup (this build only): opened in a median of 7 ms over 8 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.48
- [ar] popup from the box at 392,572 size 480x96
- [ar] popup empty: height 96, mean 44
- [ar] 'المفكرة': height 207 (empty 96), sd 33.4
- [ar] 'كروم': height 173 (empty 96), sd 34.3
- [ar] 'واي فاي': height 239 (empty 96), sd 32.0
- [ar] '250*12': height 173 (empty 96), sd 34.3
- [ar] '5 kg to lb': height 173 (empty 96), sd 35.5
- [ar] 'هجري': height 239 (empty 96), sd 32.3
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 34.3
- [ar] 'اطفاء': height 239 (empty 96), sd 31.8
- [ar] 'مشكلة': height 339 (empty 96), sd 33.7
- [ar] 'qzxwvkjh': height 207 (empty 96), sd 35.1
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.55
- [en] popup from the box at 392,572 size 480x96
- [en] popup empty: height 96, mean 44
- [en] 'notepad': height 319 (empty 96), sd 35.6
- [en] 'paint': height 439 (empty 96), sd 35.8
- [en] 'واي فاي': height 239 (empty 96), sd 34.5
- [en] '250*12': height 173 (empty 96), sd 35.8
- [en] '5 kg to lb': height 173 (empty 96), sd 37.0
- [en] 'هجري': height 239 (empty 96), sd 34.9
- [en] 'يوتيوب قرآن': height 207 (empty 96), sd 37.2
- [en] 'اطفاء': height 239 (empty 96), sd 33.7
- [en] 'مشكلة': height 339 (empty 96), sd 35.5
- [en] 'qzxwvkjh': height 207 (empty 96), sd 37.8
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.44
- [ar-light] popup empty: height 96, mean 184
- [ar-light] 'calculator': height 439 (empty 96), sd 65.0
- [ar-light] '250*12': height 173 (empty 96), sd 74.3
- [ar-light] 'مشكلة': height 339 (empty 96), sd 67.0
- [extras] Alt+Space opened the quick search
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 144 apps, 188 settings pages in 82 ms

**Failed:**

- Ctrl+Alt+G did not see the Open dialog in front
