#!/usr/bin/env bash
# Publishes built artifacts as a GitHub release that the launcher's updater accepts.
# Usage: scripts/release.sh [--appimage <AppImage>] [--macos-arm64 <zip>]
#   [--macos-x86_64 <zip>] [--macos-dmg <dmg>] <_k0t1k.exe> [notes text | notes file]
# The version comes from the workspace Cargo.toml. The signature covers
#   "_k0t1k-update\n<version>\n<hex sha256 of the file>" for each updater asset
# (see core::update::signed_message), binding it to this version. The signing key is $K0T1K_UPDATE_KEY
# (default ~/.config/k0t1k/update-signing-key.pem); it must never be copied into the repo.
set -euo pipefail

# Must match PUBLIC_KEY in packages/core/src/update.rs (raw 32 bytes, base64).
PUBLIC_KEY_B64="NrARzBayytnggNAo7TAAGOuAUyNb44vAMpDhECFZs4U="

appimage=
macos_arm64=
macos_x86_64=
macos_dmg=
while [[ ${1:-} == --* ]]; do
  case $1 in
    --appimage|--macos-arm64|--macos-x86_64|--macos-dmg)
      [[ $# -ge 2 && -n $2 && $2 != --* ]] || { echo "$1 needs a path" >&2; exit 2; }
      case $1 in
        --appimage) appimage=$2 ;;
        --macos-arm64) macos_arm64=$2 ;;
        --macos-x86_64) macos_x86_64=$2 ;;
        --macos-dmg) macos_dmg=$2 ;;
      esac
      shift 2
      ;;
    --) shift; break ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
[[ $# -ge 1 && $# -le 2 ]] || { echo "usage: $0 [--appimage <AppImage>] [--macos-arm64 <zip>] [--macos-x86_64 <zip>] [--macos-dmg <dmg>] <_k0t1k.exe> [notes text or file]" >&2; exit 2; }
exe=$1
notes=${2:-}
key=${K0T1K_UPDATE_KEY:-$HOME/.config/k0t1k/update-signing-key.pem}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

[[ -f $exe ]] || { echo "no such file: $exe" >&2; exit 1; }
for asset in "$appimage" "$macos_arm64" "$macos_x86_64" "$macos_dmg"; do
  [[ -z $asset || -f $asset ]] || { echo "no such file: $asset" >&2; exit 1; }
done
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

# Validate without extracting or executing bundle contents. Keep the extractor's limits/layout.
validate_macos_zip() {
  python3 -I - "$1" "$version" "$2" <<'PY'
import plistlib
import re
import stat
import struct
import sys
import unicodedata
import zipfile

archive, version, arch = sys.argv[1:]
root = "_k0t1k.app"
info_path = root + "/Contents/Info.plist"
exe_path = root + "/Contents/MacOS/k0t1k-app"
limit = 512 * 1024 * 1024
marker = ("k0t1k-launcher/" + version).encode()
marker_pattern = re.compile(re.escape(marker) + rb"(?=[^0-9.])")


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def extra_fields(data):
    fields = {}
    while data:
        require(len(data) >= 4, "truncated ZIP extra field")
        tag, length = struct.unpack_from("<HH", data)
        require(len(data) >= 4 + length and tag not in fields, "invalid ZIP extra field")
        # zipfile ignores alternate Unicode paths, whereas the client honors them.
        require(tag != 0x7075, "alternate Unicode ZIP paths are not supported")
        fields[tag] = data[4:4 + length]
        data = data[4 + length:]
    return fields


try:
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        require(len(entries) <= 10_000, "too many ZIP entries")
        explicit, paths, aliases = set(), {}, {}
        total = 0
        for entry in entries:
            name = entry.orig_filename
            directory = name.endswith("/")
            path = name[:-1] if directory else name
            parts = path.split("/")
            require(not any(c in path for c in "\\:\0")
                    and all(p not in ("", ".", "..", "__MACOSX") for p in parts)
                    and parts[0] == root, "unsafe ZIP path: " + repr(name))
            require(path not in explicit, "duplicate ZIP path: " + repr(name))
            explicit.add(path)
            mode = entry.external_attr >> 16 if entry.create_system == 3 else 0
            kind = stat.S_IFMT(mode)
            require(kind in (0, stat.S_IFDIR if directory else stat.S_IFREG),
                    "symlink or special ZIP entry: " + repr(name))
            require(not directory or entry.file_size == 0, "ZIP directory has data")
            require(not entry.flag_bits & 1, "encrypted ZIP entry")
            require(entry.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED),
                    "unsupported ZIP compression")
            extra_fields(entry.extra)
            bundle.fp.seek(entry.header_offset)
            local = bundle.fp.read(30)
            require(len(local) == 30, "truncated local ZIP header")
            magic, _, flags, method, time, date, crc, compressed, size, names, extras = \
                struct.unpack("<4s5H3I2H", local)
            require(magic == b"PK\x03\x04" and flags == entry.flag_bits
                    and method == entry.compress_type, "inconsistent local ZIP header")
            year, month, day, hour, minute, second = entry.date_time
            require(date == ((year - 1980) << 9 | month << 5 | day)
                    and time == (hour << 11 | minute << 5 | second // 2),
                    "inconsistent local ZIP timestamp")
            bundle.fp.seek(names, 1)  # zipfile.open checks the local filename below.
            local_extra = bundle.fp.read(extras)
            require(len(local_extra) == extras, "truncated local ZIP extra fields")
            zip64 = extra_fields(local_extra).get(1, b"")
            sizes = [size, compressed]
            for index in range(2):
                if sizes[index] == 0xFFFFFFFF:
                    require(len(zip64) >= 8, "missing local ZIP64 size")
                    sizes[index] = struct.unpack_from("<Q", zip64)[0]
                    zip64 = zip64[8:]
            for actual, expected in zip((crc, *sizes),
                                        (entry.CRC, entry.file_size, entry.compress_size)):
                require(actual == expected or (flags & 8 and actual == 0),
                        "inconsistent local ZIP checksum or size")
            for index in range(1, len(parts) + 1):
                parent = "/".join(parts[:index])
                is_directory = index < len(parts) or directory
                require(parent not in paths or paths[parent] == is_directory,
                        "file/directory collision: " + repr(name))
                paths[parent] = is_directory
                normalized = unicodedata.normalize("NFD", parent).casefold()
                require(normalized not in aliases or aliases[normalized] == parent,
                        "case/Unicode ZIP path collision: " + repr(name))
                aliases[normalized] = parent
            if path == exe_path:
                require(mode & 0o111, "bundle binary has no executable bits")
            total += entry.file_size
            require(total <= limit, "unpacked ZIP exceeds 512 MiB")
        require(paths.get(info_path) is False and paths.get(exe_path) is False,
                "missing regular Info.plist or Contents/MacOS/k0t1k-app")
        plist_data = bytearray()
        header, tail, found_marker = b"", b"", False
        unpacked = 0
        # Reading every entry also checks local names, overlapping data and CRCs in zipfile.
        for entry in entries:
            written = 0
            with bundle.open(entry) as source:
                while True:
                    chunk = source.read(min(64 * 1024, entry.file_size - written + 1))
                    if not chunk:
                        break
                    written += len(chunk)
                    unpacked += len(chunk)
                    require(written <= entry.file_size and unpacked <= limit,
                            "actual ZIP size exceeds its budget")
                    if entry.filename == info_path:
                        require(written <= 1024 * 1024, "Info.plist exceeds 1 MiB")
                        plist_data.extend(chunk)
                    if entry.filename == exe_path:
                        header = (header + chunk[:32])[:32]
                        window = tail + chunk
                        found_marker |= marker_pattern.search(window) is not None
                        tail = window[-(len(marker) + 1):]
            require(written == entry.file_size, "truncated ZIP entry")
        found_marker |= tail.endswith(marker)
        info = plistlib.loads(plist_data)
        require(isinstance(info, dict), "Info.plist must be a dictionary")
        require(info.get("CFBundleIdentifier") == "dev.k0t1k.launcher",
                "wrong CFBundleIdentifier")
        require(info.get("CFBundleExecutable") == "k0t1k-app", "wrong CFBundleExecutable")
        require(info.get("CFBundleShortVersionString") == version
                and info.get("CFBundleVersion") == version, "wrong bundle version")
        require(len(header) == 32 and header[:4] == b"\xcf\xfa\xed\xfe",
                "expected thin little-endian Mach-O64, not a universal binary")
        expected_cpu = {"arm64": 0x0100000C, "x86_64": 0x01000007}[arch]
        require(struct.unpack_from("<I", header, 4)[0] == expected_cpu,
                "bundle binary architecture does not match " + arch)
        require(found_marker, "bundle binary lacks version marker " + marker.decode())
except Exception as error:
    sys.exit(f"invalid macOS update {archive}: {error}")
PY
}

for arch in arm64 x86_64; do
  asset=$macos_arm64
  [[ $arch != x86_64 ]] || asset=$macos_x86_64
  if [[ -n $asset ]]; then
    cp "$asset" "$work/_k0t1k-macos-$arch.zip"
    validate_macos_zip "$work/_k0t1k-macos-$arch.zip" "$arch"
  fi
done
if [[ -n $macos_dmg ]]; then
  cp "$macos_dmg" "$work/_k0t1k-macos.dmg"
fi

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
for arch in arm64 x86_64; do
  if [[ -f $work/_k0t1k-macos-$arch.zip ]]; then
    sign "_k0t1k-macos-$arch.zip"
    args+=("$work/_k0t1k-macos-$arch.zip" "$work/_k0t1k-macos-$arch.zip.sig")
  fi
done
if [[ -n $macos_dmg ]]; then
  args+=("$work/_k0t1k-macos.dmg")
fi
if [[ -z $notes ]]; then
  args+=(--notes "")
elif [[ -f $notes ]]; then
  args+=(--notes-file "$notes")
else
  args+=(--notes "$notes")
fi
gh "${args[@]}"
