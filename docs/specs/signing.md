# Spec: Signed updates (minisign)

The in-app updater (`src/update.rs`) replaces the running `bahth.exe` with a file
from the latest GitHub release. This spec makes sure that file was released by
the owner, not just by whoever can edit releases. The design is the one used by
Lemon (local-ai-chat), with its own key pair and InstaFind's release flow.

<div dir="rtl">

## خطوات المالك (مرة واحدة، بدون أي جهاز ثابت)

**الوضع الآن:** التوقيع غير مُفعَّل بعد، والإصدارات تُنشر كما كانت. لا يتغير شيء حتى تكمل هذه الخطوات.
كل شيء يجري في المتصفح على GitHub، ولا يبقى أي ملف على أي جهاز.

1. **فعّل التحقق بخطوتين (2FA)** في حسابك على GitHub إن لم يكن مفعّلاً:
   الصورة الشخصية ← Settings ← Password and authentication ← Enable two-factor authentication.
2. **أنشئ البيئة `release`:** افتح المستودع ← Settings ← Environments ← New environment، واكتب الاسم `release` ثم Configure environment.
   - فعّل **Required reviewers** وأضف نفسك (`sarmadiq9090-coder`) ثم Save protection rules.
   - في **Deployment branches and tags** اختر Selected branches and tags ← Add deployment branch or tag rule ← اكتب `main`.
3. **أنشئ المفاتيح في Codespace مؤقت:** في صفحة المستودع اضغط Code ← Codespaces ← Create codespace on main. وفي الطرفية (Terminal) في الأسفل شغّل:
   ```sh
   sudo apt-get update && sudo apt-get install -y minisign
   mkdir -p ~/k && cd ~/k
   minisign -G -p primary.pub -s primary.key   # اكتب كلمة مرور قوية مرتين
   minisign -G -p backup.pub  -s backup.key    # كلمة مرور قوية أخرى
   cat primary.key; cat primary.pub; cat backup.key; cat backup.pub
   ```
   المفاتيح خاصة ببحث سريع وحده. لا تستعمل مفاتيح Lemon أو أي برنامج آخر.
4. **ضع المفتاح الأساسي في أسرار البيئة:** Settings ← Environments ← `release` ← Environment secrets ← Add environment secret:
   - `MINISIGN_SECRET_KEY`: محتوى `primary.key` كاملاً (السطران).
   - `MINISIGN_PASSWORD`: كلمة مرور المفتاح الأساسي.
5. **احفظ المفتاح الاحتياطي في مدير كلمات المرور فقط:** محتوى `backup.key` كاملاً وكلمة مروره. لا تضعه في GitHub ولا على أي جهاز.
6. **أرسل المفتاحين العامّين إلى Claude:** انسخ محتوى `primary.pub` و`backup.pub` (ليسا سرّيين) وأرسلهما في جلسة Claude. سيفتح Claude طلب دمج يضيفهما إلى `keys/` وإلى قائمة `TRUSTED_PUBLIC_KEYS` في البرنامج (انظر [keys/README.md](../../keys/README.md)).
   بين الخطوة 4 وهذا الطلب، تُوقَّع الإصدارات وتُرفع ملفات `.minisig` معها، لكن النسخ المثبّتة لا تتحقق منها بعد (تحذير في سجل GitHub).
7. **احذف الـ Codespace:** من github.com/codespaces ← (…) ← Delete.

بعد دمج طلب المفاتيح يصير التوقيع **إلزامياً**: النسخ الجديدة ترفض أي تحديث بلا توقيع صحيح.
في كل يوم إصدار يصلك إشعار من GitHub لمراجعة مهمة `sign` والموافقة عليها (Review deployments ← Approve).
وافق فقط على الإصدارات التي بدأتها أنت.

> **انتبه:** إذا وُجدت المفاتيح العامة في `keys/` وغاب السر `MINISIGN_SECRET_KEY` من البيئة، تتوقف مهمة `sign` برسالة خطأ ولا يُنشر الإصدار، لأن النسخ المثبّتة سترفضه.

</div>

## Threat model

**Problem.** The updater trusts `bahth.exe` because its SHA-256 matches the
`digest` GitHub publishes for the same release asset. Both come from the release,
so anyone who can edit releases can ship malware to every install. That covers a
stolen token or session, a collaborator account, or a compromised action in
`build.yml`.

**Defence.** Every release exe gets a minisign signature `<exe>.minisig`:
`bahth.exe.minisig`, `bahth-x86.exe.minisig` and `bahth-arm64.exe.minisig`. The
app trusts only the public keys compiled into it
(`update::TRUSTED_PUBLIC_KEYS`: a primary key and a backup key). The signature's
trusted comment must be exactly `instafind <tag>`, e.g. `instafind v1.0.0`. The
comment is covered by the signature, so an older signed exe cannot be replayed as
a newer version. The comment does not name the CPU. Another CPU's signed exe
under this CPU's asset name is refused by `validate_pe` (machine type).

| Attacker can… | Result |
|---|---|
| upload or replace release assets (token, collaborator, compromised action in `release`) | **Blocked.** They have no secret key, so no valid `.minisig`, and the update is refused. |
| re-publish an older signed exe under a new tag | **Blocked.** The trusted comment names the old tag. |
| delete the `.minisig`, or serve a broken one | **Blocked.** A missing or invalid signature means refuse. There is never a fallback. |
| push a version bump to main to start a release | `sign` **waits for the owner's approval** in the `release` Environment. |
| change the app's key list | Only through a signed release, so they need the key first. |
| compromise the **build** (a malicious dependency or action in `exe`) | **Not covered by signing**: CI signs whatever was built. Mitigated by SHA-pinned actions and Dependabot review. |
| take over the owner's GitHub account **and** approve | **Not covered.** Mitigated by 2FA and the approval being visible. |
| steal the backup key | They would need the owner's password manager. The key is not on GitHub or on any PC. |

Downgrades are never offered: the updater installs only a strictly newer version.

The update manifest on the website (`fastkiwi.app/updates/instafind/latest.json`,
docs/specs/update-manifest.md) is signed with these same keys, with the trusted
comment `instafind update-manifest <tag>`, and lists the GitHub and GitLab
addresses of each exe. Each exe still needs its own `.minisig`.

**Transition.** The key list ships **empty**. Such builds behave exactly as
before: they check the SHA-256 only (required since 0.9.1)
and log `signature checking not configured`. Once the owner has keys, a
follow-up PR pastes them in. Every build after that **requires** a valid
signature. The switch is compile-time only: there is no setting, environment
variable or server flag that skips it. Clients on older builds stay unprotected
until they update to a build with keys. That first update is checked by SHA-256
only.

## App side (`src/update.rs`)

- Crate `minisign-verify` (verify-only, no dependencies).
- `TRUSTED_PUBLIC_KEYS: &[&str]`: base64 public keys (line 2 of a `.pub` file),
  primary first. It is marked `// TODO(owner): paste the InstaFind minisign
  public keys here`.
- `parse_release_for` records `<asset>.minisig` as `Release::sig_url` if the
  release has it, and only if its URL is on GitHub (`download_host_ok`). It is
  optional there, so the check still reports an unsigned release. The web-page
  fallback (`release_from_tag`) uses the usual download address next to the exe.
- `download_and_install`: when `signature_checking_configured()` is true, it
  fetches the `.minisig` first (at most `MAX_SIG_BYTES` = 4 KB, from GitHub
  only). `SignatureCheck::new(sig, tag, keys)` then requires all of these:
  - it parses (a BOM and CRLF are tolerated);
  - it is pre-hashed (legacy signatures are refused);
  - its key id is one of the trusted keys;
  - its trusted comment is `expected_trusted_comment(tag)`.

  Then the exe is streamed through the verifier as it downloads, next to the
  SHA-256. `finalize` checks the signature and the comment's global signature.
  Any failure deletes the download, keeps the current version, and shows
  `signature_error` (ar/en, with the manual download link).
- `keys_dir_matches_trusted_keys`: `keys/*.pub` and `TRUSTED_PUBLIC_KEYS` must
  hold the same keys, so the two are always added in one PR (`keys/README.md`).
  A `.key` file in `keys/` fails it too.
- Tests (`cargo test update::`) use a committed **TEST-ONLY** key set in
  `tests/fixtures/update-signing/`. `regen.sh` regenerates it, and the secret
  halves are not kept. The tests cover:
  - a valid signature, also streamed in pieces;
  - the backup key;
  - a tampered exe;
  - another key;
  - the wrong tag or app name in the comment, and an edited comment;
  - legacy, missing and malformed signatures;
  - empty and broken key lists;
  - `.minisig` asset parsing;
  - the production list never holding a test key, and every production key
    parsing.

## Release flow (`.github/workflows/build.yml`)

A release still starts when main has a `Cargo.toml` version with no tag, and only
after every check (including `smoke` and `ui`) has passed.

| Job | Token / secrets | Does |
|---|---|---|
| `version` | `contents: read` | Reads the version, and says whether its tag is new. Releasing is skipped unless it is. |
| `sign` | Environment **`release`** (owner approval, branch `main`), with `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD`. `contents: read` | Runs **no repository code** (no checkout). It reads `keys/minisign-{primary,backup}.pub` at the commit through the API (only a 404 counts as "not committed"; any other API error fails). **No `MINISIGN_SECRET_KEY`** and no public key: it logs a warning, signs nothing and passes, so releases are published unsigned as before. **Secret, no public key yet**: it signs, checks against the public half recreated from the secret (`minisign -R`), and warns that apps do not check signatures yet. **Public keys but no secret**: it fails, since apps with those keys would refuse an unsigned update. To sign, it downloads the three smoke-tested exes and installs minisign 0.12 from a pinned URL with a pinned SHA-256 (the archive was verified with its author's key). It runs `minisign -S -t "instafind <tag>"` for each exe and checks each with `minisign -V -H` (pre-hashed only, as the app requires) against the public keys, including the exact trusted comment. Finally it uploads artifact `bahth-minisig`. |
| `release` | `contents: write`, `actions: write` | Deletes any stale draft of the tag left by a failed run, then creates the release as a **draft** with the three exes. If signed, it uploads the three `.minisig` files. It checks that the draft has exactly the expected non-empty assets, then publishes it (`--draft=false --latest`) and starts `packaging.yml`. |

Draft safety: `releases/latest`, which every installed copy asks for, never
returns a draft, and the updater also ignores drafts. A half-made release is never
offered. If `release` fails after creating the draft, re-run the failed jobs: the
stale draft is deleted first.

Compatibility: asset names are unchanged (`bahth.exe`, `bahth-x86.exe`,
`bahth-arm64.exe`). The `.minisig` files are extra assets that old clients
ignore. `packaging.yml` downloads only `bahth*.exe`. Releases end up non-draft,
non-prerelease and latest, as before.

## Rotation and loss

A client trusts only the keys compiled into the version it runs, so a release must
be signed by a key that every client still in use trusts. Keep rotations rare.

- **Primary key lost or leaked.**
  1. Make a new backup pair in a Codespace.
  2. Put the **old backup** secret key and its password into the `release`
     secrets. It is now the signing key.
  3. Release a version with `TRUSTED_PUBLIC_KEYS = [old backup, new backup]`,
     and update `keys/*.pub` to match. Every client trusts the old backup, so
     every client accepts this release.
  4. Keep the new backup in the password manager only. If the key leaked, also
     rotate GitHub credentials and review recent releases.
- **Backup key lost.** Ship `[primary, new backup]` in the next release, signed by
  the primary.
- **Both keys lost.** No release can be verified by existing clients. Users must
  download a build with the new keys **by hand**, once, from the Releases page or
  fastkiwi.app. There is deliberately no remote override.

## Verifying a release yourself

```sh
minisign -Vm bahth.exe -p keys/minisign-primary.pub   # or the backup key
#   → "Trusted comment: instafind vX.Y.Z"
```
