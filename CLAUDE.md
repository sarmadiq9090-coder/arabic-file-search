# CLAUDE.md — InstaFind (arabic-file-search)

## Owner & workflow (shared rules, same in every repo)

Owner: Sarmad (`sarmadiq9090-coder`). Writes Arabic, is not technical in English, and works only
through Claude sessions, with no fixed computer.

- Reply to the owner in Arabic. Keep code, identifiers, commit messages and PR titles in English,
  and give PR bodies an Arabic summary first.
- Never push to the default branch. Work on `claude/<topic>`, open a PR, and let the owner merge
  unless they explicitly say "ادمج" (merge).
- One Claude session per repository at a time. Before starting, check open PRs and recently pushed
  `claude/*` branches. If another session is active, coordinate through it instead of editing the
  same files.
- The owner has no fixed computer: never ask them to keep files, keys or tools on a PC. Use GitHub
  (Environments/secrets, Codespaces) or their password manager.
- **Release train (D-035, D-047, D-057).** Each app has its own day at 09:00 Asia/Baghdad (InstaFind:
  Saturday; Sun Remember Me, Mon Lemon, Tue InstaWords, Wed InstaConvert only when complete, Thu QA,
  Fri rest). D-057 (one app at a time) means a release ships when the app is finished and the owner
  says «موافق»; the day is the default slot, not a reason to wait. ALL of an app's finished, green
  features merge together (D-032/D-034); nothing else is merged. The Store update ships with the
  release. The site (fastkiwi.app) is exempt: it publishes when green (D-046). A critical security
  fix may go early with the owner's explicit approval.
- **Release standard (D-058).** The FastKiwi release standard 1.4 with the council's 11 amendments
  applies (full text: brain `core/RELEASE-STANDARD.md`). The release checklist with evidence (brain
  `core/RELEASE-CHECKLIST.md`) goes in every release PR body. CI runs on Windows Server images, so
  say "Windows 10/11 not tested on a real device" unless the owner tested. Every app must have an
  in-app update button and a report-a-problem button.
- **One GitHub writer (D-040, core/GITHUB-SAFETY.md in the brain).** Only the manager session pushes,
  opens PRs, comments, merges or dispatches workflows; other sessions work locally and report to the
  manager. Serial writes >=2 s apart (one serial writer), <=12 writes per hour, <=100 per day, <=3 new PRs per repo per day, <=2
  workflow_dispatch per repo per day, 1 rerun max. On any 403/429/abuse/CAPTCHA signal: stop all
  writes and tell the owner.
- **Closed source (D-051, 2026-11-09).** FastKiwi apps are proprietary (free to use; pricing
  undecided). Don't add "open source"/MIT wording; keep third-party notices. Code published before
  2026-11-09 stays under its MIT licence.
- **Mirror.** `.github/workflows/mirror.yml` pushes main, tags and claude/* daily to GitLab
  (`fastkiwi1/arabic-file-search`), no force, no prune; it skips until the owner adds the `GITLAB_DEPLOY_KEY` secret.
- Things only the owner can do: repo settings, deleting branches or repos, money decisions, secrets,
  merging. Ask, and give exact click-by-click steps in Arabic.
- Don't enable features that write to third-party systems, or anything security-sensitive, without
  the owner's explicit approval.
- Commit only your own files (`git add <paths>`).
- Commit trailer: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`
  and `Claude-Session: https://claude.ai/code/session_01H59LCK6b45r38j6yGz7oef`.

## What this is

InstaFind («بحث سريع», crate and exe `bahth`) finds files on Windows as you type, in Arabic or
English, and searches inside documents (PDF, Office, archives, OCR). One native Rust exe
(native-windows-gui + winapi), with a quick-search popup (Ctrl+Alt+F) and a self-updater.
Website: fastkiwi.app/arabic-file-search/ (`site/`). Docs: [README.md](README.md),
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) (build, checks, releases, code map),
[CHANGELOG.md](CHANGELOG.md), [packaging/SUBMITTING.md](packaging/SUBMITTING.md).

## Build, test, lint

```sh
cargo clippy --all-targets -- -D warnings   # lint (CI fails on any warning)
cargo test                                  # search/index/extraction tests also run on Linux
cargo test --release
cargo build --release                       # on Windows: target\release\bahth.exe
# Published exes, as CI builds them on Windows (MSVC, static CRT via .cargo/config.toml):
cargo build --release --target x86_64-pc-windows-msvc   # bahth.exe
cargo build --release --target i686-pc-windows-msvc     # bahth-x86.exe
python ci/check_imports.py bahth.exe                    # no VCRUNTIME / non-system DLL imports, DLLs from System32 only
# From Linux, a quick mingw check build only: cargo build --release --target x86_64-pc-windows-gnu
```

## CI (.github/workflows)

- `build.yml` (push to main, `v*` tags, PRs): `linux-test` (ubuntu: clippy, test, test --release),
  `windows-lint` (clippy), `windows` (test --release, x64 and 32-bit), `exe` (MSVC + static CRT on
  windows, imports checked by `ci/check_imports.py`), `exe-arm64` (windows-11-arm, same), `smoke`
  (each exe on real Windows, `ci/smoke.ps1`), `ui` (real-user test, `ci/ui-test.ps1`), then `release`.
- `release-notes.yml`: edits a published release's title/body when `releases/*-notes.md` changes.
- `packaging.yml`: rewrites `bucket/`, winget and Chocolatey manifests after a release and commits
  them to main. Pushing to Chocolatey/winget is off until `CHOCOLATEY_API_KEY` / `WINGET_PAT` exist.
- `pages.yml`: deploys `site/` to GitHub Pages on push to main (paths `site/**`, `assets/**`).
- `media.yml`, `quick-shots.yml`: manual, or on `claude/**` pushes that touch their scripts; they
  commit results to `docs/media/` and `docs/quick-shots/` on that branch.

## Release (automatic on a version bump)

`build.yml`'s `release` job publishes as soon as main has a `Cargo.toml` version with no tag yet
and every job (including `smoke` and `ui`) passed. So bump the version only on the app's train day (Sat).
Steps (docs/DEVELOPMENT.md → Releases): bump `version` in `Cargo.toml`, add the CHANGELOG entry,
write `releases/vX.Y.Z-notes.md` (first line `# title`, then Arabic body, then a short **English:**
line), merge to main. The job tags `vX.Y.Z`, uploads the exes and starts `packaging.yml`.

Must not break:
- Asset names `bahth.exe` (x64; every installed copy since 0.3.0 looks for it), `bahth-x86.exe`,
  `bahth-arm64.exe` (`src/update.rs`); `SHA256SUMS.txt` is an additional asset, keep it. Releases stay non-draft, non-prerelease, marked latest.
- The repository URL `sarmadiq9090-coder/arabic-file-search` (hard-coded in `src/update.rs`, the
  manifests, and the Scoop bucket users added).
- Data in `%LOCALAPPDATA%\BahthSari` and `HKCU\Software\BahthSari`.
- `site/` changes go live on merge (`pages.yml`): merge them only on the app's train day.

## Do not move or rename

- `ci/` (scripts called by the workflows: smoke, ui-test, check_imports, bench, demo, make-media, quick-shots).
- `bucket/` (this repo is a Scoop bucket: `bucket/instafind.json`; tafket's docs/PACKAGING.md
  also points users at this bucket).
- `releases/*.md` (read by `release-notes.yml` and by `build.yml`'s release job).
- `packaging/` (`update-manifests.py`, winget and Chocolatey sources used by `packaging.yml`).
- `docs/media/`, `docs/quick-shots/` (linked from README), `vendor/native-windows-gui`
  (patched crate, see vendor/README.md), `assets/` (icon used by `build.rs`).

## Pending for the owner

- Optional secrets for package managers: `CHOCOLATEY_API_KEY`, `WINGET_PAT`
  (see `packaging.yml` and packaging/SUBMITTING.md). Ask before adding: both publish to third parties.
