# Speed: this build against 0.6.1

Measured by `ci/quick-shots.ps1` on a GitHub Actions runner: Microsoft Windows Server 2025 Datacenter, AMD EPYC 9V74 80-Core Processor, 2026-11-02. This build: bahth 0.8.2. Both exes search the same saved index (150 MB); medians of 5 runs, taken alternately.

## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)

| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |
|---|---:|---:|---:|---:|
| `report` | 597 | 597 | +0% | 2091 / 2091 |
| `notepad` | 598 | 595 | -1% | 48 / 48 |
| `احمد` | 590 | 593 | +0% | 0 / 0 |
| `تقرير مالي` | 595 | 596 | +0% | 1 / 1 |
| `مشكلة` | 601 | 645 | +7% | 2 / 2 |
| `ext:dll` | 639 | 648 | +1% | 111585 / 111585 |
| `system32 config` | 596 | 599 | +0% | 0 / 0 |
| **all** | **4,217** | **4,272** | **+1%** | |

## Start-up of the main window (index already saved)

Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program's own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.

| Phase (median ms) | 0.6.1 | this build | change |
|---|---:|---:|---:|
| Shown | 30 | 32 | +6% |
| Built | 111 | 103 | -7% |
| Layout | 111 | 103 | -7% |
| FirstTick | 283 | 279 | -1% |

Raw phases (last run of each):

- 0.6.1: `single-instance=8 init=10 font=10 settings=10 window=32 list=64 controls=80 tray=87 events=88 layout=110 built=110 loop=110 first-tick=282`
- this build: `single-instance=9 init=11 font=11 settings=11 window=34 list=63 controls=78 tray=84 events=84 layout=105 built=106 loop=106 first-tick=278`

Quick search popup (this build only): opened in a median of 5 ms over 50 openings (`quick search shown in` in log.txt).

## Checks

- [ar] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar] floating box idle vs hovered: mean difference 2.48
- [ar] popup from the box at 392,572 size 480x96
- [ar] box still opens the popup after 46 rapid clicks
- [ar] popup empty: height 96, mean 44
- [ar] 'المفكرة': height 207 (empty 96), sd 33.1
- [ar] 'كروم': height 173 (empty 96), sd 34.4
- [ar] 'واي فاي': height 239 (empty 96), sd 32.0
- [ar] '250*12': height 173 (empty 96), sd 34.4
- [ar] '5 kg to lb': height 173 (empty 96), sd 35.6
- [ar] 'هجري': height 239 (empty 96), sd 32.4
- [ar] 'يوتيوب قرآن': height 207 (empty 96), sd 34.1
- [ar] 'اطفاء': height 239 (empty 96), sd 31.8
- [ar] 'مشكلة': height 485 (empty 96), sd 34.4
- [ar] 'qzxwvkjh': height 207 (empty 96), sd 34.8
- [en] floating box at 392,676 size 236x36 on a 1024x768 screen
- [en] floating box idle vs hovered: mean difference 2.55
- [en] popup from the box at 392,572 size 480x96
- [en] box still opens the popup after 46 rapid clicks
- [en] popup empty: height 96, mean 44
- [en] 'notepad': height 319 (empty 96), sd 35.5
- [en] 'paint': height 439 (empty 96), sd 35.6
- [en] 'واي فاي': height 239 (empty 96), sd 34.6
- [en] '250*12': height 173 (empty 96), sd 35.9
- [en] '5 kg to lb': height 173 (empty 96), sd 37.1
- [en] 'هجري': height 239 (empty 96), sd 34.9
- [en] 'يوتيوب قرآن': height 207 (empty 96), sd 36.9
- [en] 'اطفاء': height 239 (empty 96), sd 33.7
- [en] 'مشكلة': height 485 (empty 96), sd 35.6
- [en] 'qzxwvkjh': height 207 (empty 96), sd 37.5
- [ar-light] floating box at 392,676 size 236x36 on a 1024x768 screen
- [ar-light] floating box idle vs hovered: mean difference 8.44
- [ar-light] popup empty: height 96, mean 184
- [ar-light] 'calculator': height 439 (empty 96), sd 65.3
- [ar-light] '250*12': height 173 (empty 96), sd 74.2
- [ar-light] 'مشكلة': height 485 (empty 96), sd 64.7
- [extras] Alt+Space opened the quick search
- [extras] Ctrl+Alt+G: quick switch: hotkey, file dialog found (box found)
- [extras] Enter on 'device manager' started: mmc
- [extras] Enter on 'devices and printers' started: ApplicationFrameHost, dllhost, svchost, SystemSettings
- [extras] hidden mode, cursor away: a click on the box's place reaches 'Progman'
- catalog: launcher catalog: 145 apps, 188 settings pages in 85 ms

All checks passed.
