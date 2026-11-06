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
- **Weekly release train.**
  - All improvements, additions and changes to a published app ship to users ONCE a week, together.
    This covers new versions and tags, website (fastkiwi.app) updates, and package-manager
    (winget/Scoop/Chocolatey) manifest updates.
  - Release day is Friday 09:00 (Asia/Baghdad). The weekly health check in the owner's manager
    session runs just before it and prepares the train; the owner approves it.
  - A release goes out only after CI is green on Linux and Windows.
  - During the week, work goes to `claude/*` branches or PRs, or is merged to the default branch
    WITHOUT bumping the version or pushing a tag.
  - arabic-file-search's build.yml publishes a release automatically when the Cargo.toml version
    changes on main, so bump the version only on release day.
  - github.io deploys on every push to main, so merge site PRs only on release day.
  - Exceptions: mawared-desktop, and new apps that have never been published. These may release
    whenever needed.
  - A critical security fix may go out early, only with the owner's explicit approval.
- Things only the owner can do: repo settings, deleting branches or repos, money decisions, secrets,
  merging. Ask, and give exact click-by-click steps in Arabic.
- Don't enable features that write to third-party systems, or anything security-sensitive, without
  the owner's explicit approval.
- Commit only your own files (`git add <paths>`).
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

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
python ci/check_imports.py bahth.exe                    # no VCRUNTIME / non-system DLL imports
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
and every job (including `smoke` and `ui`) passed. So bump the version only on release day.
Steps (docs/DEVELOPMENT.md → Releases): bump `version` in `Cargo.toml`, add the CHANGELOG entry,
write `releases/vX.Y.Z-notes.md` (first line `# title`, then Arabic body, then a short **English:**
line), merge to main. The job tags `vX.Y.Z`, uploads the exes and starts `packaging.yml`.

Must not break:
- Asset names `bahth.exe` (x64; every installed copy since 0.3.0 looks for it), `bahth-x86.exe`,
  `bahth-arm64.exe` (`src/update.rs`). Releases stay non-draft, non-prerelease, marked latest.
- The repository URL `sarmadiq9090-coder/arabic-file-search` (hard-coded in `src/update.rs`, the
  manifests, and the Scoop bucket users added).
- Data in `%LOCALAPPDATA%\BahthSari` and `HKCU\Software\BahthSari`.
- `site/` changes go live on merge (`pages.yml`): merge them only on release day.

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
