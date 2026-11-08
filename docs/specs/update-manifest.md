# Spec: Update manifest on fastkiwi.app (`fastkiwi.app/updates/<app>/`)

Installed apps must keep getting updates even when GitHub is down or the
owner's account is unavailable (council rule 13). Owner decision D-038: free
services only, so there is **no Cloudflare R2** and no `updates.fastkiwi.app`.
Instead:

- **The manifest** is served by the website on Cloudflare Pages (free):
  `https://fastkiwi.app/updates/<app>/latest.json` and
  `https://fastkiwi.app/updates/<app>/latest.json.minisig`, a small file signed
  with the **same minisign keys** as the programs (docs/specs/signing.md).
- **The programs** stay where they are: GitHub Releases first, then the same
  file as a release asset of the project's GitLab copy
  (`https://gitlab.com/fastkiwi1/<repo>/-/releases/<tag>/downloads/<name>`),
  a **fallback that is populated later**. Until it is, the GitLab address
  answers 404 and the GitHub copy is the only one.

This spec is shared by InstaFind (`instafind`), Lemon (`local-ai-chat`) and
Remember Me (`remember-me`); each repository's copy ends with its own
section. The tool `tools/make-update-manifest` is the same file in all three
repositories.

<div dir="rtl">

## ملخّص للمالك

- البرنامج يسأل أولاً ملف التحديث على موقعنا `fastkiwi.app/updates/…` (Cloudflare Pages، مجاني، بلا بطاقة ولا R2)، وإن لم يجد جواباً سليماً يرجع إلى GitHub كما كان.
- البرنامج نفسه يُنزَّل من GitHub أولاً، ثم من نسخة احتياطية على GitLab (`gitlab.com/fastkiwi1/…`) ستُملأ لاحقاً.
- لا يتغيّر شيء للمستخدمين قبل أن يُنشر الملف على الموقع: البرنامج يجد العنوان غير متاح فيرجع إلى GitHub بصمت.
- ما دامت مفاتيح التوقيع غير مضافة، لا يُستعمل ملف الموقع إلا إذا طابقت بصمته (SHA-256) ما ينشره GitHub للإصدار نفسه.
- لا نوافذ في الفحص التلقائي. في الفحص اليدوي تبقى رسالة «تعذّر الوصول إلى خادم التحديثات الآن…» إذا تعذّر المصدران.
- لا رجوع إلى إصدار أقدم أبداً.

**ما يحدث عند كل إصدار:** CI يكتب `latest.json` (ويوقّعه حين يوجد السر) ويضعه في artifact اسمه `site-updates`، ثم ينشره الموقع (مستودع `sarmadiq9090-coder.github.io`) مع نشره التالي. CI هنا لا يرفع شيئاً إلى أي خدمة.
**خطواتك لاحقاً:** لا شيء الآن. ملء نسخة GitLab الاحتياطية سيأتي في PR منفصل يحتاج موافقتك (يكتب إلى خدمة خارجية).

</div>

## The manifest

`https://fastkiwi.app/updates/<app>/latest.json`, UTF-8 JSON, at most 64 KB:

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
        "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v9.9.9/bahth.exe",
        "https://gitlab.com/fastkiwi1/arabic-file-search/-/releases/v9.9.9/downloads/bahth.exe"
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
- `urls` are tried **in order**: the GitHub release first, then the GitLab
  release asset (fallback, populated later). Only `https` addresses whose last
  path segment is `name` are used; each app also limits the hosts to GitHub and
  its own GitLab project's releases (`gitlab.com/fastkiwi1/<repo>/-/releases/`).
- An asset's minisign signature, when the release is signed, is
  `<url>.minisig` next to **each** URL (so the GitLab release holds the
  `.minisig` files too, once it is populated).
- Extra fields are ignored by the apps; new fields are additive only.

### Signature

`latest.json.minisig` next to the manifest: a detached, **pre-hashed**
minisign signature of the exact bytes of `latest.json`, made with the app's
release key (primary, or the backup in an emergency). Its trusted comment is
`<app> update-manifest <tag>`, e.g. `instafind update-manifest v9.9.9`. This
differs from the programs' comment (`instafind v9.9.9`), so one signature can
never be passed off as the other.

## Updater flow (InstaFind and Lemon)

1. **Fetch** `https://fastkiwi.app/updates/<app>/latest.json`. Any failure (no
   network, DNS, TLS, HTTP 403, 404, 429, 5xx, too large, not valid for this
   app/CPU) means: use the current GitHub feed, exactly as before.
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
   unless GitHub knows a newer release (a manifest that was not updated yet);
   if GitHub cannot be reached then, the answer is still "up to date".
4. **Download** from each URL in order (GitHub, then GitLab). Every copy is
   checked against the manifest's size and SHA-256, and, when keys exist,
   against the minisign signature (also tried from each `<url>.minisig` in
   order). A failed or altered copy is deleted and the next URL is tried.
5. **Never downgrade.** Only a strictly newer version is offered or installed.

Automatic checks never show a window; a failed manual check shows the neutral
"The update server can't be reached right now…" text (unchanged).

### Threat model

| Attacker can… | Result |
|---|---|
| change `latest.json` on the website (site repo, Cloudflare Pages account) | **Blocked** with keys: no valid `latest.json.minisig`, and each program still needs its own `.minisig`. Without keys: the manifest is used only when GitHub reports the same checksum. |
| change files on the GitLab copy (GitLab account) | **Blocked**: every download is checked against the manifest's checksum (and, with keys, its `.minisig`); an altered copy is deleted and never run. Only `gitlab.com/fastkiwi1/<repo>/-/releases/…` addresses are accepted. |
| serve an old signed manifest (replay) | **No downgrade**: an old version is never offered. A frozen manifest only delays an update while GitHub is also down; while GitHub is up, a newer GitHub release wins. |
| block `fastkiwi.app` | The app falls back to GitHub, as before. |
| take GitHub down (or the account) | With keys, the signed manifest on fastkiwi.app and the GitLab copies keep updates going (once GitLab is populated). Without keys, nothing can confirm the manifest, so updates wait for GitHub (fail closed). |

## Building, signing and staging (`tools/make-update-manifest`)

Python 3.8+, standard library only (signing calls the `minisign` command). It
never uploads anything.

```sh
tools/make-update-manifest build --app instafind --version 9.9.9 \
    --repo sarmadiq9090-coder/arabic-file-search \
    --asset x64=dist/bahth.exe --asset x86=dist/bahth-x86.exe --asset arm64=dist/bahth-arm64.exe \
    --notes-file releases/v9.9.9-notes.md --out latest.json
tools/make-update-manifest sign latest.json        # MINISIGN_SECRET_KEY / MINISIGN_PASSWORD; skips without them
tools/make-update-manifest verify latest.json --pub keys/minisign-primary.pub --assets dist
tools/make-update-manifest stage latest.json --out site-updates
tools/make-update-manifest verify https://fastkiwi.app/updates/instafind/latest.json --download
```

- `build` hashes the files and writes the URLs in order:
  `https://github.com/<repo>/releases/download/<tag>/<name>`, then
  `https://gitlab.com/fastkiwi1/<repo name>/-/releases/<tag>/downloads/<name>`
  (`--gitlab-project group/name` to change it, `--no-gitlab` for GitHub only).
- `sign` writes `latest.json.minisig` with the trusted comment above. Without
  the secret it prints a warning and exits 0 (`--require` makes that an error).
- `verify` checks the schema, the signature (with `--pub`), local files
  (`--assets`) and every download URL (`--download`; the GitLab URL reports
  HTTP 404 until the copy is populated).
- `stage` validates the manifest and copies it, with `latest.json.minisig`
  when there is one, to `<out>/updates/<app>/`. A stale signature left in that
  folder is removed, so an unsigned manifest never sits next to an old
  signature.
- Tests: `python3 -m unittest discover -s tools -p "test_*.py"` (fake v9.9.9,
  a local HTTP server; the signing tests need `minisign`).

## Hand-off to the website

The release workflow **uploads nothing** to any host. After the release is
public it runs `stage` and uploads the folder as the workflow artifact
`site-updates`:

```
updates/instafind/latest.json
updates/instafind/latest.json.minisig          (when signed)
```

The website repository (`sarmadiq9090-coder.github.io`, deployed to Cloudflare
Pages) picks that artifact up in its own deploy, copies `updates/instafind/` into
the site and publishes it at `https://fastkiwi.app/updates/instafind/`. That deploy
is configured in the site repository, not here. Serve `latest.json*` with
`Cache-Control: max-age=300` or less. `latest.json` (and its signature) are
also attached to the GitHub release, so the site can be rebuilt from there.
The programs stay on GitHub Releases; the GitLab copy is populated later, by a
separate change that needs the owner's approval (it writes to a third-party
system).

## InstaFind

- `src/update.rs`: `MANIFEST_URL` (`https://fastkiwi.app/updates/instafind/latest.json`),
  `MANIFEST_SIG_URL`, `parse_manifest_for`, `verify_manifest_signature`,
  `resolve_update` (the flow above, over the `UpdateSources` trait so it is
  tested on Linux), `first_that_works` (URLs in order). Hosts: GitHub, or
  `gitlab.com` under `/fastkiwi1/arabic-file-search/-/releases/` with no `..`
  segment and no `%` escape (`update_host_ok`, `GITLAB_RELEASES_PATH`). The
  GitHub API path still accepts GitHub only (`download_host_ok`).
- `check_latest(log)` and `download_and_install(rel, progress, log)` log which
  source was used and every URL that failed (`log.txt`).
- Tests (`cargo test update::`): fixtures `tests/fixtures/update-signing/manifest.json*`
  (a fake v9.9.9 made by the tool and signed with the TEST-ONLY keys,
  `regen.sh`), and a fake update server keyed by URL that answers with bodies
  or HTTP errors. Covered: the signed manifest while GitHub is down; bad,
  missing, foreign and replayed signatures; a manifest that is down or returns
  403/404/5xx; the no-keys cross-check with GitHub (same, different, missing
  checksum; GitHub down); no downgrade; a stale manifest; hostile manifests
  (other hosts, other GitLab projects, `..`, ports, user info, queries); URL
  order.
- CI (`build.yml`): job `manifest` (no secrets) builds `latest.json` from the
  smoke-tested exes and checks it with `verify --assets`. `sign` checks with
  `jq` that it names this tag and the SHA-256s of the exes it signs, then signs
  it (`instafind update-manifest <tag>`) when it signs the exes, i.e. only when
  `MINISIGN_SECRET_KEY` exists. `release` attaches `latest.json` and, when
  signed, `latest.json.minisig` to the release, then stages them as the
  `site-updates` artifact.
