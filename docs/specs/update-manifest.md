# Spec: Update manifest on Cloudflare (`updates.fastkiwi.app`)

Installed apps must keep getting updates even when GitHub is down or the
owner's account is unavailable (council rule 13). fastkiwi.app itself is on
GitHub Pages, so the primary update feed lives on Cloudflare instead:
`https://updates.fastkiwi.app/<app>/latest.json`, a small manifest signed with
the **same minisign keys** as the programs (docs/specs/signing.md). GitHub
Releases stay the fallback.

This spec is shared by InstaFind (`instafind`), Lemon (`local-ai-chat`) and
Remember Me (`remember-me`). The tool `tools/make-update-manifest` is the same
file in all three repositories.

<div dir="rtl">

## ملخّص للمالك

- البرنامج يسأل أولاً `updates.fastkiwi.app` (على Cloudflare)، وإن لم يجد جواباً سليماً يرجع إلى GitHub كما كان.
- لا يتغيّر شيء للمستخدمين قبل أن تجهّز `updates.fastkiwi.app`: البرنامج يجد العنوان غير متاح فيرجع إلى GitHub بصمت.
- ما دامت مفاتيح التوقيع غير مضافة، لا يُستعمل ملف Cloudflare إلا إذا طابقت بصمته (SHA-256) ما ينشره GitHub للإصدار نفسه.
- لا نوافذ في الفحص التلقائي. في الفحص اليدوي تبقى رسالة «تعذّر الوصول إلى خادم التحديثات الآن…» إذا تعذّر المصدران.
- لا رجوع إلى إصدار أقدم أبداً.

**خطواتك لاحقاً (مرة واحدة):** أنشئ على Cloudflare حاوية R2 واربطها بالنطاق الفرعي `updates.fastkiwi.app`، ثم اطلب من Claude
أن يضيف خطوة رفع الملفات إليها في CI (تحتاج سرّاً من Cloudflare، ولن تُفعَّل دون موافقتك).
حتى ذلك الحين تُرفق كل نسخة `latest.json` (و`latest.json.minisig` عند التوقيع) بإصدار GitHub، ويمكن نسخها يدوياً.

</div>

## The manifest

`https://updates.fastkiwi.app/<app>/latest.json`, UTF-8 JSON, at most 64 KB:

```json
{
  "schema": 1,
  "app": "instafind",
  "version": "9.9.9",
  "tag": "v9.9.9",
  "released": "2026-11-13T06:00:00Z",
  "notes_url": "https://fastkiwi.app/releases/",
  "notes": "short release notes (plain text, at most 2000 characters)",
  "assets": {
    "x64": {
      "name": "bahth.exe",
      "size": 4321000,
      "sha256": "64 lower-case hex digits",
      "sha512": "base64 of the 64-byte SHA-512 (what electron-updater uses)",
      "urls": [
        "https://updates.fastkiwi.app/instafind/v9.9.9/bahth.exe",
        "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v9.9.9/bahth.exe"
      ]
    },
    "x86":   { "name": "bahth-x86.exe", "...": "..." },
    "arm64": { "name": "bahth-arm64.exe", "...": "..." }
  }
}
```

- `version` is `MAJOR.MINOR.PATCH` (no pre-releases), `tag` is `v<version>`.
- `assets` keys are `x64`, `x86`, `arm64`. Each app reads only its own CPU's
  entry, and only if `name` is the asset name it expects (asset names never
  change).
- `urls` are tried **in order**: the Cloudflare R2 mirror first, then the
  GitHub release. Only `https` addresses whose last path segment is `name` are
  used; each app also limits the hosts (see below).
- An asset's minisign signature, when the release is signed, is
  `<url>.minisig` next to **each** URL (so the mirror holds the `.minisig`
  files too).

### Signature

`latest.json.minisig` next to the manifest: a detached, **pre-hashed**
minisign signature of the exact bytes of `latest.json`, made with the app's
release key (primary, or the backup in an emergency). Its trusted comment is
`<app> update-manifest <tag>`, e.g. `instafind update-manifest v9.9.9`. This
differs from the programs' comment (`instafind v9.9.9`), so one signature can
never be passed off as the other.

## Updater flow

1. **Fetch** `latest.json`. Any failure (no network, DNS, TLS, HTTP 403, 404,
   429, 5xx, too large, not valid for this app/CPU) means: use the current
   GitHub feed, exactly as before.
2. **Trust it.**
   - With keys compiled in (`TRUSTED_PUBLIC_KEYS` not empty): fetch
     `latest.json.minisig` and verify it (pre-hashed, a trusted key, the
     comment above). Any failure: skip the manifest, use GitHub.
   - While `TRUSTED_PUBLIC_KEYS` is **empty** (today): the manifest is used
     only if the GitHub feed reports the **same tag with the same SHA-256** for
     this CPU's asset. GitHub unreachable, another release, or another
     checksum: skip the manifest and use GitHub's answer.
3. **Offer** a trusted manifest whose version is strictly newer than the
   running one. If a trusted manifest is not newer, the app is up to date
   unless GitHub knows a newer release (a mirror that was not updated yet);
   if GitHub cannot be reached then, the answer is still "up to date".
4. **Download** from each URL in order. Every copy is checked against the
   manifest's size and SHA-256, and, when keys exist, against the minisign
   signature (also tried from each `<url>.minisig` in order). A failed or
   altered copy is deleted and the next URL is tried.
5. **Never downgrade.** Only a strictly newer version is offered or installed.

Automatic checks never show a window; a failed manual check shows the neutral
"The update server can't be reached right now…" text (unchanged).

### Threat model (additions to docs/specs/signing.md)

| Attacker can… | Result |
|---|---|
| change files on the Cloudflare mirror (R2 token, Cloudflare account) | **Blocked** with keys: no valid `latest.json.minisig`, and each program still needs its own `.minisig`. Without keys: the manifest is used only when GitHub reports the same SHA-256. |
| serve an old signed manifest (replay) | **No downgrade**: an old version is never offered. A frozen mirror only delays an update while GitHub is also down; while GitHub is up, a newer GitHub release wins. |
| block `updates.fastkiwi.app` | The app falls back to GitHub, as before. |
| take GitHub down (or the account) | With keys, the signed manifest and the R2 copies keep updates going. Without keys, nothing can confirm the manifest, so updates wait for GitHub (fail closed). |

## Building and signing (`tools/make-update-manifest`)

Python 3.8+, standard library only (signing calls the `minisign` command).

```sh
tools/make-update-manifest build --app instafind --version 9.9.9 \
    --repo sarmadiq9090-coder/arabic-file-search \
    --asset x64=dist/bahth.exe --asset x86=dist/bahth-x86.exe --asset arm64=dist/bahth-arm64.exe \
    --notes-file releases/v9.9.9-notes.md --out latest.json
tools/make-update-manifest sign latest.json        # MINISIGN_SECRET_KEY / MINISIGN_PASSWORD; skips without them
tools/make-update-manifest verify latest.json --pub keys/minisign-primary.pub --assets dist
tools/make-update-manifest verify https://updates.fastkiwi.app/instafind/latest.json --download
```

- `build` hashes the files and writes the URLs: mirror
  `https://updates.fastkiwi.app/<app>/<tag>/<name>` (`--layout flat`:
  `<app>/<name>`), then `https://github.com/<repo>/releases/download/<tag>/<name>`.
- `sign` writes `latest.json.minisig` with the trusted comment above. Without
  the secret it prints a warning and exits 0 (`--require` makes that an error).
- `verify` checks the schema, the signature (with `--pub`), local files
  (`--assets`) and every download URL (`--download`). Use it after setting up
  or changing the mirror.
- Tests: `python3 -m unittest discover -s tools -p "test_*.py"` (fake v9.9.9,
  a local HTTP server; the signing tests need `minisign`).

## Mirror layout (Cloudflare R2 behind `updates.fastkiwi.app`)

```
instafind/latest.json
instafind/latest.json.minisig            (when signed)
instafind/v9.9.9/bahth.exe               (+ .minisig when signed)
instafind/v9.9.9/bahth-x86.exe           (+ .minisig)
instafind/v9.9.9/bahth-arm64.exe         (+ .minisig)
```

Upload the versioned files first and `latest.json` (+ `.minisig`) last, so the
manifest never points at a file that is not there yet. Keep old versions: a
client may still be downloading one. Serve with `Cache-Control: max-age=300`
or less for `latest.json*`.

Uploading to R2 is **not wired yet**: it needs a Cloudflare API token, a
secret that writes to a third-party system, so it waits for the owner's
approval. Until then the release attaches `latest.json` (and
`latest.json.minisig` when signed) to the GitHub release, and the apps fall
back to GitHub while `updates.fastkiwi.app` does not answer.

## InstaFind

- `src/update.rs`: `MANIFEST_URL`, `MANIFEST_SIG_URL`, `parse_manifest_for`,
  `verify_manifest_signature`, `resolve_update` (the flow above, over the
  `UpdateSources` trait so it is tested on Linux), `first_that_works` (URLs in
  order). Hosts: GitHub, or `updates.fastkiwi.app` (`update_host_ok`). The
  GitHub API path still accepts GitHub only (`download_host_ok`).
- `check_latest(log)` and `download_and_install(rel, progress, log)` log which
  source was used and every URL that failed (`log.txt`).
- Tests (`cargo test update::`): fixtures `tests/fixtures/update-signing/manifest.json*`
  (a fake v9.9.9 made by the tool and signed with the TEST-ONLY keys,
  `regen.sh`), and a fake update server keyed by URL that answers with bodies
  or HTTP errors. Covered: the signed manifest while GitHub is down; bad,
  missing, foreign and replayed signatures; a mirror that is down or returns
  403/404/5xx; the no-keys cross-check with GitHub (same, different, missing
  checksum; GitHub down); no downgrade; a stale mirror; hostile manifests; URL
  order.
- CI (`build.yml`): job `manifest` (no secrets) builds `latest.json` from the
  smoke-tested exes and checks it with `verify --assets`. `sign` checks with
  `jq` that it names this tag and the SHA-256s of the exes it signs, then signs
  it (`instafind update-manifest <tag>`) when it signs the exes, i.e. only when
  `MINISIGN_SECRET_KEY` exists. `release` attaches `latest.json` and, when
  signed, `latest.json.minisig`.
