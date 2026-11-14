#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="$ROOT/.nami-signing-identity"
NAME="Nami Local Development"
KEYCHAIN="$(security default-keychain -d user | tr -d '"' | sed 's/^ *//')"
if [[ -f "$CONFIG" ]]; then
  IDENTITY="$(cat "$CONFIG")"
  if [[ ! "$IDENTITY" =~ ^[[:xdigit:]]{40}$ ]] || ! security find-identity -p codesigning | /usr/bin/grep -F "$IDENTITY" >/dev/null; then
    echo "固定签名身份不可用，请恢复原证书和私钥；不会生成替代身份。" >&2
    exit 1
  fi
  echo "Nami 固定本地签名身份已可用。"
  exit 0
fi
umask 077
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/nami-certificate.XXXXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
security find-certificate -a -c "$NAME" -p "$KEYCHAIN" > "$SCRATCH/certificate.pem" 2>/dev/null || true
if [[ ! -s "$SCRATCH/certificate.pem" ]]; then
  cat > "$SCRATCH/signing.cnf" <<'CERTIFICATE'
[req]
distinguished_name = name
[name]
[signing]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CERTIFICATE
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
    -subj "/CN=Nami Local Development/O=Nami Local Development" \
    -keyout "$SCRATCH/Nami Local Development.key" -out "$SCRATCH/certificate.pem" \
    -extensions signing -config "$SCRATCH/signing.cnf" 2>/dev/null
  security import "$SCRATCH/Nami Local Development.key" -k "$KEYCHAIN" -T /usr/bin/codesign
  security import "$SCRATCH/certificate.pem" -k "$KEYCHAIN"
fi
if [[ "$(/usr/bin/grep -c 'BEGIN CERTIFICATE' "$SCRATCH/certificate.pem")" != 1 ]]; then
  echo "存在多张 Nami 开发证书，停止自动选择。" >&2
  exit 1
fi
IDENTITY="$(/usr/bin/openssl x509 -in "$SCRATCH/certificate.pem" -noout -fingerprint -sha1 | sed 's/.*=//;s/://g')"
if ! security find-identity -p codesigning | /usr/bin/grep -F "$IDENTITY" >/dev/null; then
  echo "证书私钥不可用，请恢复原证书和私钥，不要生成替代证书。" >&2
  exit 1
fi
printf '%s\n' "$IDENTITY" > "$CONFIG"
echo "已固定 Nami 专用本地签名。请保留钥匙串中的证书和私钥。"
