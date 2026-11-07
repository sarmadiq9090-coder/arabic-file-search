#!/bin/sh
# TEST-ONLY minisign fixtures for the update-signature unit tests in src/update.rs.
#
# These keys are PUBLIC on purpose and protect nothing: they are used only under
# #[cfg(test)]. A unit test asserts that none of them is ever in the production
# key list `update::TRUSTED_PUBLIC_KEYS`. NEVER use them to sign a release; the
# real InstaFind keys are made as described in docs/specs/signing.md.
#
# Regenerate (needs `minisign`, e.g. `apt install minisign`), from this folder:
#   sh regen.sh
set -eu
cd "$(dirname "$0")"
rm -f ./*.pub ./*.key ./*.minisig manifest.json
for k in test-primary test-backup test-other; do
  minisign -G -W -f -p "$k.pub" -s "$k.key" >/dev/null
done
printf 'InstaFind update-signature test payload (not a real exe).\n' > payload.bin
sign() { # <key> <sig file> <trusted comment> [extra minisign flags]
  key=$1; out=$2; tc=$3; shift 3
  minisign -S "$@" -s "$key.key" -m payload.bin -x "$out" -t "$tc" >/dev/null
}
sign test-primary payload.bin.minisig               "instafind v9.9.9"
sign test-backup  payload.bin.backup.minisig        "instafind v9.9.9"
sign test-other   payload.bin.other-key.minisig     "instafind v9.9.9"
sign test-primary payload.bin.wrong-version.minisig "instafind v9.9.8"
sign test-primary payload.bin.wrong-name.minisig    "local-ai-chat v9.9.9"
sign test-primary payload.bin.legacy.minisig        "instafind v9.9.9" -l
# The update manifest (docs/specs/update-manifest.md) for a fake v9.9.9 whose
# three programs are payload.bin, made by the real tool and signed the same way.
tmp=$(mktemp -d)
for exe in bahth.exe bahth-x86.exe bahth-arm64.exe; do cp payload.bin "$tmp/$exe"; done
python3 ../../../tools/make-update-manifest build --app instafind --version 9.9.9 \
  --repo sarmadiq9090-coder/arabic-file-search --released 2026-11-13T06:00:00Z \
  --notes-url https://fastkiwi.app/releases/ \
  --asset x64="$tmp/bahth.exe" --asset x86="$tmp/bahth-x86.exe" --asset arm64="$tmp/bahth-arm64.exe" \
  --out manifest.json >/dev/null
rm -r "$tmp"
msign() { # <key> <sig file> <trusted comment> [extra minisign flags]
  key=$1; out=$2; tc=$3; shift 3
  minisign -S "$@" -s "$key.key" -m manifest.json -x "$out" -t "$tc" >/dev/null
}
msign test-primary manifest.json.minisig               "instafind update-manifest v9.9.9"
msign test-backup  manifest.json.backup.minisig        "instafind update-manifest v9.9.9"
msign test-other   manifest.json.other-key.minisig     "instafind update-manifest v9.9.9"
msign test-primary manifest.json.wrong-version.minisig "instafind update-manifest v9.9.8"
msign test-primary manifest.json.exe-comment.minisig   "instafind v9.9.9"
msign test-primary manifest.json.legacy.minisig        "instafind update-manifest v9.9.9" -l
# The secret halves are not needed by the tests: drop them (a rerun makes new ones).
rm -f ./*.key
echo "fixtures regenerated"
