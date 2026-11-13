#!/usr/bin/env bash
# InstaFind update-signing keys: the one-time key ceremony.
#
# The OWNER runs this once, in a temporary GitHub Codespace of this repository
# (it works from a phone browser), then deletes the Codespace:
#
#     bash keys/ceremony.sh
#
# It installs minisign 0.12 (the same pinned, checksum-verified file the
# `sign` job in .github/workflows/build.yml uses), makes the primary and the
# backup key pair, each with a strong random password, checks that both work,
# and prints four labelled blocks:
#   A  MINISIGN_SECRET_KEY  -> secret of the GitHub Environment `release`
#   B  MINISIGN_PASSWORD    -> secret of the GitHub Environment `release`
#   C  backup secret key + its password -> the owner's password manager ONLY
#   D  both PUBLIC keys     -> send to Claude (not secret)
# Each block is also saved as ~/k/A.txt ... ~/k/D.txt, so it can be opened in
# the editor (`code ~/k/A.txt`) and copied from there.
#
# Everything it writes stays in ~/k (never in the repository, never in
# ~/.minisign). Deleting the Codespace deletes it. Full steps, in Arabic:
# docs/specs/signing.md. Never run it in CI or on a shared machine.
set -euo pipefail
umask 077

K="$HOME/k"
MINISIGN_URL="https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-linux.tar.gz"
MINISIGN_SHA256="9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73"

say() { printf '%s\n' "$@"; }
die() {
  say "" "!!! $1" "" >&2
  exit 1
}

if [ -e "$K/primary.key" ] || [ -e "$K/backup.key" ]; then
  die "توجد مفاتيح سابقة في ~/k. لن أكتب فوقها. / Keys already exist in ~/k; not overwriting them."
fi
mkdir -p "$K"
cd "$K"

# ---------------------------------------------------------------- minisign
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) die "Unsupported CPU: $(uname -m)" ;;
esac
say "تنزيل minisign 0.12 والتحقق من بصمته… / Downloading minisign 0.12 and checking its checksum…"
curl -sSfL -o minisign.tar.gz "$MINISIGN_URL"
echo "${MINISIGN_SHA256}  minisign.tar.gz" | sha256sum -c - >/dev/null ||
  die "بصمة minisign لا تطابق. توقّف. / The minisign download does not match its checksum. Stopped."
tar xzf minisign.tar.gz "minisign-linux/$arch/minisign" 2>/dev/null
M="$K/minisign-linux/$arch/minisign"
"$M" -v >/dev/null

# ---------------------------------------------------------------- the keys
# 24 random bytes as 48 hex characters (192 bits): letters and digits only,
# so nothing is lost when the password is copied on a phone.
new_password() { od -An -N24 -tx1 /dev/urandom | tr -d ' \n'; }

make_key() { # <name> <password>
  printf '%s\n%s\n' "$2" "$2" | "$M" -G -p "$K/$1.pub" -s "$K/$1.key" >/dev/null 2>&1 ||
    die "تعذّر إنشاء المفتاح $1. / Could not make the $1 key."
}

# Signs a test file exactly as CI does and verifies it as the app does
# (pre-hashed, with the trusted comment).
check_key() { # <name> <password>
  printf 'InstaFind key check\n' >"$K/check.txt"
  printf '%s\n' "$2" | "$M" -S -s "$K/$1.key" -m "$K/check.txt" -x "$K/check.txt.minisig" -t "instafind check" >/dev/null 2>&1 ||
    die "تعذّر التوقيع بالمفتاح $1. / Signing with the $1 key failed."
  [ "$("$M" -V -H -Q -m "$K/check.txt" -x "$K/check.txt.minisig" -p "$K/$1.pub")" = "instafind check" ] ||
    die "فشل التحقق من المفتاح $1. / The $1 key did not verify."
  rm -f "$K/check.txt" "$K/check.txt.minisig"
}

primary_pw=$(new_password)
backup_pw=$(new_password)
if [ ${#primary_pw} -ne 48 ] || [ ${#backup_pw} -ne 48 ] || [ "$primary_pw" = "$backup_pw" ]; then
  die "Could not make random passwords."
fi
say "إنشاء المفتاحين… / Making the two keys…"
make_key primary "$primary_pw"
make_key backup "$backup_pw"
check_key primary "$primary_pw"
check_key backup "$backup_pw"

# ---------------------------------------------------------------- the blocks
cat "$K/primary.key" >"$K/A.txt"
printf '%s\n' "$primary_pw" >"$K/B.txt"
{
  say "InstaFind BACKUP minisign secret key (keep in the password manager only):"
  cat "$K/backup.key"
  say "" "Password of the backup key:" "$backup_pw"
} >"$K/C.txt"
{
  say "keys/minisign-primary.pub:"
  cat "$K/primary.pub"
  say "" "keys/minisign-backup.pub:"
  cat "$K/backup.pub"
} >"$K/D.txt"

line="=================================================================="
block() { # <letter> <Arabic title> <English title> <file>
  say "" "$line" "  [$1]  $2" "       $3" "$line" ">>>>> $1 START (copy the lines below / انسخ الأسطر التالية)"
  cat "$4"
  say "<<<<< $1 END (stop copying here / توقّف عن النسخ هنا)"
}

clear 2>/dev/null || true
say "تم إنشاء المفتاحين والتحقق منهما. / Both keys were made and checked." \
  "" \
  "انسخ كل كتلة بين سطري START و END فقط (بدون السطرين نفسيهما)." \
  "Copy each block between its START and END lines only (not those lines)." \
  "يمكنك أيضاً فتح كل كتلة في المحرر:  code ~/k/A.txt  (أو B أو C أو D)" \
  "You can also open each block in the editor:  code ~/k/A.txt  (or B, C, D)"

block A "السر MINISIGN_SECRET_KEY (سطران). ضعه في أسرار البيئة release على GitHub." \
  "MINISIGN_SECRET_KEY (two lines): a secret of the GitHub Environment 'release'." "$K/A.txt"
block B "السر MINISIGN_PASSWORD. ضعه في أسرار البيئة release على GitHub." \
  "MINISIGN_PASSWORD: a secret of the GitHub Environment 'release'." "$K/B.txt"
block C "المفتاح الاحتياطي وكلمة مروره: في مدير كلمات المرور فقط. لا تضعه في GitHub ولا ترسله لأحد." \
  "Backup secret key and its password: your password manager ONLY. Never GitHub, never send it." "$K/C.txt"
block D "المفتاحان العامّان (ليسا سرّيين): أرسلهما إلى Claude." \
  "Both PUBLIC keys (not secret): send them to Claude." "$K/D.txt"

say "" "$line" \
  "بعد أن تنسخ الكتل الأربع: احذف هذا الـ Codespace (github.com/codespaces ← … ← Delete)." \
  "لا ترسل أبداً الكتل A أو B أو C لأي أحد، ولا لـ Claude." \
  "When the four blocks are copied, DELETE this Codespace (github.com/codespaces -> ... -> Delete)." \
  "Never send blocks A, B or C to anyone, Claude included." \
  "$line"
