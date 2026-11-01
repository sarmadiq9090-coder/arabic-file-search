# Publishing InstaFind to package managers

All manifests come from one script, `packaging/update-manifests.py`
(`version sha256 date`). After each release, CI runs it and commits the result
to main (`Packaging: update manifests for vX.Y.Z`):

| File | Used by |
| --- | --- |
| `bucket/instafind.json` | Scoop (this repo is a bucket) |
| `packaging/winget/manifests/s/Sarmad/InstaFind/<version>/` | winget, copy into `microsoft/winget-pkgs` |
| `packaging/chocolatey/` | Chocolatey community repository |

The x64 exe asset must stay named `bahth.exe`: the in-app updater of every
installed copy and all three manifests download `releases/download/vX.Y.Z/bahth.exe`.
Since v0.7.0 each release also has `bahth-x86.exe` (32-bit) and
`bahth-arm64.exe`. Scoop and winget list all three CPUs; Chocolatey lists x64
and x86 (it has no ARM64 URL, and Windows on ARM runs the x64 exe).

Points that apply to every package manager:

- **Self-update.** InstaFind can replace its own exe (Options → check for
  updates, or the daily prompt). It replaces the real file inside the package
  manager's folder, so it keeps working, but the package manager still records
  the old version until its own upgrade runs. Upgrading through the package
  manager is always safe.
- **Data** lives outside the package: `%LOCALAPPDATA%\BahthSari` and
  `HKCU\Software\BahthSari`. No uninstall removes it. The "Start with
  Windows" entry (`HKCU\...\Run\BahthSari`) is removed by the Scoop and
  Chocolatey uninstall scripts. winget has no uninstall hook, so it stays and
  points at a missing file until the user turns it off in Options first.
- **Unsigned exe.** No store requires code signing. Antivirus scans do apply
  (see winget below).

---

## winget (microsoft/winget-pkgs)

The manifests use schema **1.12.0**, the version winget-pkgs currently
recommends (its PR template and ValidationFailureGuide; 1.10.0 is also
accepted). They are multi-file manifests (version, defaultLocale en-US, locale
ar-SA, installer), which is required; singleton manifests are not accepted.
`InstallerType: portable` is the correct type for a single exe and is
supported. winget copies it to
`%LOCALAPPDATA%\Microsoft\WinGet\Packages\Sarmad.InstaFind_...\`, links the
command `bahth` (the first entry of `Commands`; a portable install only
gets one command, so only `bahth` is listed) and registers it in
Apps & features.

Once per account: sign the Microsoft CLA. The bot asks on your first PR;
the PR cannot merge until you sign.

### Option A: wingetcreate (easiest)

```powershell
winget install Microsoft.WingetCreate
wingetcreate token --store                 # or pass --token <PAT> below
winget validate --manifest packaging\winget\manifests\s\Sarmad\InstaFind\0.6.0
winget settings --enable LocalManifestFiles   # as admin, once
winget install --manifest packaging\winget\manifests\s\Sarmad\InstaFind\0.6.0
winget uninstall Sarmad.InstaFind
wingetcreate submit packaging\winget\manifests\s\Sarmad\InstaFind\0.6.0
```

`submit` forks winget-pkgs for you, pushes a branch and opens the PR. The PAT
is a classic GitHub token with `public_repo` scope.

Automatic option: store that PAT as the repository secret `WINGET_PAT`. Then
`packaging.yml` runs `wingetcreate submit` for each release it handles.

### Option B: pull request by hand

1. Fork https://github.com/microsoft/winget-pkgs and create a branch.
2. Copy `packaging/winget/manifests/s/Sarmad/InstaFind/<version>/` to the same
   path in the fork (`manifests/s/Sarmad/InstaFind/<version>/`).
3. Open a PR with the title `New package: Sarmad.InstaFind version 0.6.0`
   (later: `Update: Sarmad.InstaFind to 0.7.0`), fill in the checklist, and
   change only one manifest per PR.

### What the validation pipeline does, and the risks

- Checks schema and URLs (HTTPS, reachable, no poor SmartScreen *URL*
  reputation; GitHub release URLs are fine). Downloads the exe, verifies the
  SHA-256, scans it with several antivirus engines and runs a PUA check, then
  installs it in a clean VM and runs it.
- **Unsigned apps are allowed.** The hard rule is antivirus: if any engine
  flags the exe (as malware or as PUA), the PR is rejected whatever the app
  is. If that happens, submit `bahth.exe` as a false positive at
  https://www.microsoft.com/wdsi/filesubmission, quoting the PR URL, and ask
  for `@wingetbot run`. Its own updater and HTTP server (phone search, off by
  default) are things scanners sometimes notice, so keep the release notes
  clear about them.
- After validation a moderator reviews and merges, usually within days. Then
  `winget install Sarmad.InstaFind` (or `winget install instafind`, the moniker)
  works.
- SmartScreen on the user's PC: winget does not sign the file, so users
  may still see a SmartScreen prompt on first launch until the exe has built
  up reputation. Signing (for example through SignPath's free plan for open
  source projects) is the only way to remove that prompt.
- winget checks that a portable file is unchanged before it uninstalls it.
  After an in-app self-update the exe's hash differs, so
  `winget uninstall Sarmad.InstaFind` may refuse and ask for `--force`.
  `winget upgrade` to the next manifest fixes this.

Later versions: either let `packaging.yml` (with `WINGET_PAT`) submit, or run
`wingetcreate update Sarmad.InstaFind --version X.Y.Z --urls
https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/vX.Y.Z/bahth.exe --submit`.

---

## Chocolatey (community.chocolatey.org)

See `packaging/chocolatey/README.md`. In short: create an account on
community.chocolatey.org, copy the API key, set `<owners>` in
`update-manifests.py` to your user name, then run `choco pack` + `choco push`
(or add the secret `CHOCOLATEY_API_KEY` so CI pushes). The first versions wait
for human moderation. The package id `instafind` must still be free: search
community.chocolatey.org before the first push.

---

## Scoop

### Now: this repository is a bucket

```powershell
scoop bucket add instafind https://github.com/sarmadiq9090-coder/arabic-file-search
scoop install instafind/instafind
```

This gives the `bahth` and `instafind` commands and a Start menu shortcut
"InstaFind". CI keeps `bucket/instafind.json` current. `checkver`/`autoupdate`
are also set, so `scoop update instafind` works. Nothing is persisted, because
the app keeps no data in its own folder.

### Later: the official Extras bucket

ScoopInstaller/Extras takes package requests through issues. Its template
requires:

- the app to be "reasonably well-known and widely used (e.g. if it's a GitHub
  project, it should have at least **100 stars and/or 50 forks**)";
- an English interface or English documentation (met);
- the latest stable version, ideally the full version with a standard install
  (met: version-specific URL, only a tiny uninstall script).

**InstaFind does not qualify yet**: the repository has 1 star and 0 forks
(2026-11-01). Once it passes about 100 stars:

1. Open a "Package Request" issue in ScoopInstaller/Extras with the
   popularity numbers.
2. After approval, fork Extras, add `bucket/instafind.json` (this file, with
   4-space indentation, in the field order from their CONTRIBUTING guide), and
   open a PR titled `instafind: Add version X.Y.Z`. Then comment `/verify`.
3. After it merges, their Excavator updates the version through
   `checkver`/`autoupdate`. Version-bump PRs are not accepted there.

(The Main bucket is for CLI developer tools with at least 500 stars and 150
forks, so it does not fit.)
