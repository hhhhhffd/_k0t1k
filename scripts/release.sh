#!/usr/bin/env bash
# Publishes a built _k0t1k.exe as a GitHub release that the launcher's updater accepts.
# Usage: scripts/release.sh <path to _k0t1k.exe> [notes text | notes file]
# The version comes from the workspace Cargo.toml. The signing key is $K0T1K_UPDATE_KEY
# (default ~/.config/k0t1k/update-signing-key.pem); it must never be copied into the repo.
set -euo pipefail

# Must match PUBLIC_KEY in packages/core/src/update.rs (raw 32 bytes, base64).
PUBLIC_KEY_B64="NrARzBayytnggNAo7TAAGOuAUyNb44vAMpDhECFZs4U="

[[ $# -ge 1 ]] || { echo "usage: $0 <_k0t1k.exe> [notes text or file]" >&2; exit 2; }
exe=$1
notes=${2:-}
key=${K0T1K_UPDATE_KEY:-$HOME/.config/k0t1k/update-signing-key.pem}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

[[ -f $exe ]] || { echo "no such file: $exe" >&2; exit 1; }
[[ -f $key ]] || { echo "signing key not found: $key" >&2; exit 1; }
version=$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f&&/^version[[:space:]]*=/{gsub(/[" ]/,"",$3);print $3;exit}' "$root/Cargo.toml")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "cannot read a X.Y.Z version from Cargo.toml (got '$version')" >&2; exit 1; }
tag="v$version"

if gh release view "$tag" >/dev/null 2>&1; then
  echo "release $tag already exists" >&2
  exit 1
fi
if git -C "$root" rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists locally" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$exe" "$work/_k0t1k.exe"
openssl pkeyutl -sign -rawin -inkey "$key" -in "$work/_k0t1k.exe" -out "$work/sig.bin"
base64 -w0 "$work/sig.bin" > "$work/_k0t1k.exe.sig"

# Verify against the embedded public key: wrap the raw key in an Ed25519 SubjectPublicKeyInfo header.
{ printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; printf '%s' "$PUBLIC_KEY_B64" | base64 -d; } > "$work/pub.der"
openssl pkey -pubin -inform DER -in "$work/pub.der" -out "$work/pub.pem"
if ! openssl pkeyutl -verify -rawin -pubin -inkey "$work/pub.pem" -in "$work/_k0t1k.exe" -sigfile "$work/sig.bin" >/dev/null; then
  echo "the signature does not verify against the embedded public key; wrong signing key?" >&2
  exit 1
fi

args=(release create "$tag" "$work/_k0t1k.exe#_k0t1k.exe" "$work/_k0t1k.exe.sig" --title "_k0t1k $version")
if [[ -z $notes ]]; then
  args+=(--notes "")
elif [[ -f $notes ]]; then
  args+=(--notes-file "$notes")
else
  args+=(--notes "$notes")
fi
gh "${args[@]}"
