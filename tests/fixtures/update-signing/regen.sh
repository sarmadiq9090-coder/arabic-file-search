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
rm -f ./*.pub ./*.key ./*.minisig
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
# The secret halves are not needed by the tests: drop them (a rerun makes new ones).
rm -f ./*.key
echo "fixtures regenerated"
