# keys/: InstaFind's update-signing public keys

This folder holds the **public** halves of InstaFind's minisign keys:

- `minisign-primary.pub`: signs every release in CI (`build.yml`, job `sign`).
- `minisign-backup.pub`: used only if the primary key is lost or leaked.

It is empty on purpose until the owner makes the keys with [`ceremony.sh`](ceremony.sh)
(run once in a temporary Codespace: `bash keys/ceremony.sh`, steps in
[docs/specs/signing.md](../docs/specs/signing.md)). **The owner sends block D**, the two
public keys (they are not secret), to a Claude session, and one pull request
then does both of these together:

1. adds `keys/minisign-primary.pub` and `keys/minisign-backup.pub` here, and
2. pastes their second lines (the base64 key starting with `RW`) into
   `TRUSTED_PUBLIC_KEYS` in `src/update.rs`, primary first.

The unit test `update::tests::keys_dir_matches_trusted_keys` fails if one is done
without the other. From the first release built with keys, installed copies
refuse any update without a valid `<exe>.minisig`, and the `sign` job fails if
the `MINISIGN_SECRET_KEY` secret is missing.

Never put a secret key (`.key`, or the `MINISIGN_SECRET_KEY` value) in this
folder or anywhere in the repository. The primary secret key lives only in the
`release` Environment secrets; the backup secret key only in the owner's
password manager. The test-only keys are in `tests/fixtures/update-signing/` and
must never appear here. Full design, threat model and key rotation:
[docs/specs/signing.md](../docs/specs/signing.md).

<div dir="rtl">

## للمالك

هذا المجلد للمفتاحين **العامّين** فقط (ملفّا `.pub`، وهما ليسا سرّيين). بعد أن تنشئ المفاتيح
بالسكربت `keys/ceremony.sh` والخطوات المكتوبة في [docs/specs/signing.md](../docs/specs/signing.md)، **أرسل الكتلة D
(المفتاحين العامّين) إلى Claude** في أي جلسة، وسيفتح طلب دمج يضعهما هنا وفي البرنامج معاً.
لا ترسل أبداً الكتل A وB وC (ملف `.key` أو كلمة المرور): السر الأساسي مكانه أسرار بيئة `release` على GitHub،
والاحتياطي مكانه مدير كلمات المرور فقط.

</div>
