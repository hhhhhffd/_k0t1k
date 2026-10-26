#!/usr/bin/env bash
# Publishes a built _k0t1k.exe as a GitHub release that the launcher's updater accepts.
# Usage: scripts/release.sh [--appimage <path to the AppImage>] <path to _k0t1k.exe> [notes text | notes file]
# The version comes from the workspace Cargo.toml. The signature covers
#   "_k0t1k-update\n<version>\n<hex sha256 of the file>" for each file (the exe and the AppImage)
# (see core::update::signed_message), so the exe cannot be republished under another version. The signing key is $K0T1K_UPDATE_KEY
# (default ~/.config/k0t1k/update-signing-key.pem); it must never be copied into the repo.
set -euo pipefail

# Must match PUBLIC_KEY in packages/core/src/update.rs (raw 32 bytes, base64).
PUBLIC_KEY_B64="NrARzBayytnggNAo7TAAGOuAUyNb44vAMpDhECFZs4U="

appimage=
if [[ ${1:-} == --appimage ]]; then
  [[ $# -ge 2 ]] || { echo "--appimage needs a path" >&2; exit 2; }
  appimage=$2
  shift 2
fi
[[ $# -ge 1 ]] || { echo "usage: $0 [--appimage <AppImage>] <_k0t1k.exe> [notes text or file]" >&2; exit 2; }
exe=$1
notes=${2:-}
key=${K0T1K_UPDATE_KEY:-$HOME/.config/k0t1k/update-signing-key.pem}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

[[ -f $exe ]] || { echo "no such file: $exe" >&2; exit 1; }
[[ -z $appimage || -f $appimage ]] || { echo "no such file: $appimage" >&2; exit 1; }
[[ -f $key ]] || { echo "signing key not found: $key" >&2; exit 1; }
version=$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f&&/^version[[:space:]]*=/{gsub(/[" ]/,"",$3);print $3;exit}' "$root/Cargo.toml")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "cannot read a X.Y.Z version from Cargo.toml (got '$version')" >&2; exit 1; }
tag="v$version"

# The exe must be a build of this version: the updater's user agent string embeds it.
if ! grep -aqF "k0t1k-launcher/$version" "$exe"; then
  echo "$exe does not contain k0t1k-launcher/$version: it was not built from this version" >&2
  exit 1
fi
# The release is tagged at HEAD, so HEAD must be the committed, published source of the exe.
if [[ -n $(git -C "$root" status --porcelain --untracked-files=no) ]]; then
  echo "the working tree has uncommitted changes" >&2
  exit 1
fi
head=$(git -C "$root" rev-parse HEAD)
upstream=$(git -C "$root" rev-parse -q --verify '@{upstream}') || { echo "the current branch has no upstream; push it first" >&2; exit 1; }
if ! git -C "$root" merge-base --is-ancestor "$head" "$upstream"; then
  echo "HEAD $head is not pushed to the upstream branch" >&2
  exit 1
fi

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

# The AppImage's binary is compressed inside it; extract it to check the version like the exe's.
if [[ -n $appimage ]]; then
  cp "$appimage" "$work/_k0t1k-x86_64.AppImage"
  chmod +x "$work/_k0t1k-x86_64.AppImage"
  (cd "$work" && ./_k0t1k-x86_64.AppImage --appimage-extract usr/bin/k0t1k-app >/dev/null)
  if ! grep -aqF "k0t1k-launcher/$version" "$work/squashfs-root/usr/bin/k0t1k-app"; then
    echo "$appimage does not contain k0t1k-launcher/$version: it was not built from this version" >&2
    exit 1
  fi
  rm -rf "$work/squashfs-root"
fi

# Verify against the embedded public key: wrap the raw key in an Ed25519 SubjectPublicKeyInfo header.
{ printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; printf '%s' "$PUBLIC_KEY_B64" | base64 -d; } > "$work/pub.der"
openssl pkey -pubin -inform DER -in "$work/pub.der" -out "$work/pub.pem"

# Writes <file>.sig next to the file in $work and checks it against the embedded public key.
sign() {
  local file=$work/$1
  local hash
  hash=$(sha256sum "$file" | cut -d' ' -f1)
  printf '_k0t1k-update\n%s\n%s' "$version" "$hash" > "$work/message"
  openssl pkeyutl -sign -rawin -inkey "$key" -in "$work/message" -out "$work/sig.bin"
  if ! openssl pkeyutl -verify -rawin -pubin -inkey "$work/pub.pem" -in "$work/message" -sigfile "$work/sig.bin" >/dev/null; then
    echo "the signature does not verify against the embedded public key; wrong signing key?" >&2
    exit 1
  fi
  base64 -w0 "$work/sig.bin" > "$file.sig"
}

cp "$exe" "$work/_k0t1k.exe"
sign _k0t1k.exe
args=(release create "$tag" "$work/_k0t1k.exe#_k0t1k.exe" "$work/_k0t1k.exe.sig" --title "_k0t1k $version" --target "$head")
if [[ -n $appimage ]]; then
  sign _k0t1k-x86_64.AppImage
  args+=("$work/_k0t1k-x86_64.AppImage" "$work/_k0t1k-x86_64.AppImage.sig")
fi
if [[ -z $notes ]]; then
  args+=(--notes "")
elif [[ -f $notes ]]; then
  args+=(--notes-file "$notes")
else
  args+=(--notes "$notes")
fi
gh "${args[@]}"
