# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 9V45 96-Core Processor, 2026-11-02. This build: bahth 0.8.1. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 493 | 473 | -4% | 2091 / 2091 |
| `notepad` | 495 | 486 | -2% | 48 / 48 |
| `احمد` | 486 | 473 | -2% | 0 / 0 |
| `تقرير مالي` | 487 | 484 | -1% | 1 / 1 |
| `مشكلة` | 474 | 486 | +3% | 2 / 2 |
| `ext:dll` | 523 | 529 | +1% | 111584 / 111584 |
| `system32 config` | 494 | 490 | -1% | 0 / 0 |
| **all** | **3,452** | **3,422** | **-1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 27 | 29 | +10% |
| Built | 90 | 83 | -8% |
| Layout | 90 | 83 | -8% |
| FirstTick | 268 | 264 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=7 init=9 font=9 settings=9 window=29 list=51 controls=62 tray=68 events=69 layout=84 built=84 loop=84 first-tick=263`
- this build: `single-instance=7 init=9 font=9 settings=9 window=26 list=50 controls=60 tray=64 events=64 layout=78 built=78 loop=78 first-tick=260`

Quick search popup (this build only): opened in a median of 5 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.48
- [ar] popup from the box at 392,572 size 480x96
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 96, mean 44
- [ar] 'المفكرة': height 207 (empty 96), sd 33.2
- [ar] 'كروم': height 173 (empty 96), sd 34.3
- [ar] 'واي فاي': height 239 (empty 96), sd 32.0
- [ar] '250*12': height 173 (empty 96), sd 34.2
- [ar] '5 kg to lb': height 173 (empty 96), sd 35.5
- [ar] 'هجري': height 239 (empty 96), sd 32.3
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 34.2
- [ar] 'اطفاء': height 239 (empty 96), sd 31.7
- [ar] 'مشكلة': height 485 (empty 96), sd 34.5
- [ar] 'qzxwvkjh': height 207 (empty 96), sd 34.9
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.55
- [en] popup from the box at 392,572 size 480x96
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 96, mean 44
- [en] 'notepad': height 319 (empty 96), sd 35.5
- [en] 'paint': height 439 (empty 96), sd 35.7
- [en] 'واي فاي': height 239 (empty 96), sd 34.5
- [en] '250*12': height 173 (empty 96), sd 35.7
- [en] '5 kg to lb': height 173 (empty 96), sd 37.0
- [en] 'هجري': height 239 (empty 96), sd 35.0
- [en] 'يوتيوب قرآن': height 207 (empty 96), sd 37.1
- [en] 'اطفاء': height 239 (empty 96), sd 33.7
- [en] 'مشكلة': height 485 (empty 96), sd 35.7
- [en] 'qzxwvkjh': height 207 (empty 96), sd 37.7
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.44
- [ar-light] popup empty: height 96, mean 184
- [ar-light] 'calculator': height 439 (empty 96), sd 65.1
- [ar-light] '250*12': height 173 (empty 96), sd 74.2
- [ar-light] 'مشكلة': height 485 (empty 96), sd 64.5
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: 
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 64 ms

**Failed:**

- Enter on 'device manager' did not start mmc.exe
- Esc did not close the popup (tool devmgmt)
