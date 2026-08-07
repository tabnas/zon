#!/usr/bin/env bash
#
# fetch-zigzon.sh — build the ZON conformance corpus from the Zig reference
# implementation, at a PINNED version. Idempotent: re-running is cheap and safe.
#
# NOTHING THIS SCRIPT DOWNLOADS OR GENERATES IS EVER COMMITTED. Everything lands
# under test/zigzon/vendor/ and test/zigzon/cases.json, both .gitignore'd. Only
# this script and test/zigzon/tools/* are tracked. (Same pattern as
# toml/.gitignore -> ts/test/toml-test/ and xml/scripts/fetch-xml-suite.sh.)
#
# ---------------------------------------------------------------------------
# WHY A GENERATED CORPUS
#
# ZON has no standalone third-party conformance suite: there is no `zon-test`,
# no spec.json, no committee fixtures. The ONLY authority on what ZON means is
# the Zig reference implementation itself. So the corpus is assembled
# mechanically from the pinned Zig release:
#
#   * test/behavior/zon/*.zon              upstream ZON behaviour documents
#   * test/cases/compile_errors/zon/*.zon  upstream ZON must-fail documents
#   * build.zig.zon and friends            real-world manifests
#   * lib/std/zon/parse.zig                ZON source literals in `test` blocks
#
# and every document's VERDICT and EXPECTED VALUE come from running that same
# pinned reference implementation over it (tools/oracle.zig), never from this
# repo's own parser. A corpus adjudicated by the thing under test would measure
# nothing.
#
# ---------------------------------------------------------------------------
# THE PINS
#
# Zig moved its canonical repository from GitHub to Codeberg during 0.16 dev
# (ziglang/zig commit 738d2be "README: migrated to codeberg"), and the GitHub
# mirror's tags stop at 0.15.2. So the corpus is pinned as the 0.16.0 release
# SOURCE TARBALL, verified by SHA-256 — a content hash is a strictly stronger
# pin than a git ref, and it is immune to the repo moving again. The
# corresponding upstream git commit is recorded below for provenance.
#
# The oracle toolchain is pinned to the SAME 0.16.0 release, so the stdlib that
# adjudicates the corpus is exactly the stdlib the corpus ships with.
set -euo pipefail

ZIG_VERSION="0.16.0"

# Upstream git provenance (canonical repo, post-migration). Recorded for the
# record; the actual fetch is the content-addressed tarball below.
ZIG_UPSTREAM_REPO="https://codeberg.org/ziglang/zig"
ZIG_UPSTREAM_TAG="0.16.0"
ZIG_UPSTREAM_COMMIT="24fdd5b7a4c1c8b5deb5b56756b9dbc8e08c86a8"

# Corpus: the release source tarball (contains lib/std/zon + test/**/zon).
ZIG_SRC_URL="https://ziglang.org/download/${ZIG_VERSION}/zig-${ZIG_VERSION}.tar.xz"
ZIG_SRC_SHA256="43186959edc87d5c7a1be7b7d2a25efffd22ce5807c7af99067f86f99641bfdf"

# Oracle toolchain: prebuilt zig 0.16.0, per host platform.
zig_toolchain_pin() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    arm64) arch=aarch64 ;;
    amd64) arch=x86_64 ;;
  esac
  case "$os-$arch" in
    Linux-x86_64)
      echo "zig-x86_64-linux-${ZIG_VERSION} 70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00" ;;
    Linux-aarch64)
      echo "zig-aarch64-linux-${ZIG_VERSION} ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17" ;;
    Darwin-aarch64)
      echo "zig-aarch64-macos-${ZIG_VERSION} b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489" ;;
    Darwin-x86_64)
      echo "zig-x86_64-macos-${ZIG_VERSION} 0387557ed1877bc6a2e1802c8391953baddba76081876301c522f52977b52ba7" ;;
    # Git Bash / MSYS on Windows runners. Zig ships a .zip there, not a .tar.xz.
    MINGW*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64)
      echo "zig-x86_64-windows-${ZIG_VERSION} 68659eb5f1e4eb1437a722f1dd889c5a322c9954607f5edcf337bc3684a75a7e zip" ;;
    MINGW*-aarch64|MSYS*-aarch64|CYGWIN*-aarch64)
      echo "zig-aarch64-windows-${ZIG_VERSION} aee38316ee4111717900f45dd3130145c39289e105541d737eb8c5ed653c78ef zip" ;;
    *)
      echo "" ;;
  esac
}

# Unpack a .tar.xz or a .zip into a directory. No silent fallbacks: if the host
# has no usable extractor this fails loudly rather than leaving the suite
# without a corpus.
unpack() {
  local archive="$1" dest="$2"
  case "$archive" in
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -q -o "$archive" -d "$dest"
      elif command -v powershell >/dev/null 2>&1; then
        powershell -NoProfile -Command \
          "Expand-Archive -Force -LiteralPath '$archive' -DestinationPath '$dest'"
      else
        die "need unzip or powershell to unpack $archive"
      fi ;;
    *)
      tar -xJf "$archive" -C "$dest" ;;
  esac
}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZIGZON="$HERE/test/zigzon"
VENDOR="$ZIGZON/vendor"
TOOLS="$ZIGZON/tools"
CASES="$ZIGZON/cases.json"

SRC_DIR="$VENDOR/zig-${ZIG_VERSION}"
TC_DIR="$VENDOR/toolchain"

mkdir -p "$VENDOR"

say() { printf '[fetch-zigzon] %s\n' "$*" >&2; }
die() { printf '[fetch-zigzon] ERROR: %s\n' "$*" >&2; exit 1; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum   >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else die "need sha256sum or shasum to verify the pinned download"
  fi
}

# download <url> <dest> <sha256>: idempotent + verified. A file already present
# with the right hash is left alone; a wrong hash is a hard failure, never a
# silent re-download of whatever the server is serving today.
download() {
  local url="$1" dest="$2" want="$3" got
  if [[ -f "$dest" ]]; then
    got="$(sha256_of "$dest")"
    if [[ "$got" == "$want" ]]; then say "cached $(basename "$dest")"; return 0; fi
    say "cached $(basename "$dest") has wrong hash, refetching"
    rm -f "$dest"
  fi
  say "downloading $url"
  curl -fsSL --retry 3 -o "$dest.part" "$url" || die "download failed: $url"
  got="$(sha256_of "$dest.part")"
  [[ "$got" == "$want" ]] || die "SHA-256 mismatch for $url
  expected $want
  actual   $got
This is a pinned artifact. A mismatch means the pin is wrong or the
download was tampered with -- it is NOT something to work around."
  mv "$dest.part" "$dest"
}

# ---------------------------------------------------------------- 1. corpus
if [[ -f "$SRC_DIR/.fetched" ]]; then
  say "corpus source already extracted at $SRC_DIR"
else
  download "$ZIG_SRC_URL" "$VENDOR/zig-${ZIG_VERSION}.tar.xz" "$ZIG_SRC_SHA256"
  say "extracting corpus source (this takes a moment)"
  rm -rf "$SRC_DIR"
  # Only the paths the extractor reads — no need to unpack 22MB of compiler.
  tar -xJf "$VENDOR/zig-${ZIG_VERSION}.tar.xz" -C "$VENDOR" \
      "zig-${ZIG_VERSION}/lib/std/zon" \
      "zig-${ZIG_VERSION}/test/behavior/zon" \
      "zig-${ZIG_VERSION}/test/cases/compile_errors" \
      "zig-${ZIG_VERSION}/build.zig.zon" \
      "zig-${ZIG_VERSION}/lib/init" \
      "zig-${ZIG_VERSION}/src/codegen" 2>/dev/null || \
  tar -xJf "$VENDOR/zig-${ZIG_VERSION}.tar.xz" -C "$VENDOR"
  touch "$SRC_DIR/.fetched"
fi

# ------------------------------------------------------------- 2. toolchain
ZIG_BIN="${ZIGZON_ZIG:-}"
if [[ -z "$ZIG_BIN" ]]; then
  pin="$(zig_toolchain_pin)"
  [[ -n "$pin" ]] || die "no pinned zig ${ZIG_VERSION} build for $(uname -s)/$(uname -m).
Set ZIGZON_ZIG=/path/to/zig (must be ${ZIG_VERSION}) and re-run."
  read -r tc_name tc_sha tc_kind <<<"$pin"
  tc_ext="tar.xz"; tc_exe=""
  if [[ "${tc_kind:-}" == "zip" ]]; then tc_ext="zip"; tc_exe=".exe"; fi
  ZIG_BIN="$TC_DIR/$tc_name/zig$tc_exe"
  if [[ -x "$ZIG_BIN" ]]; then
    say "toolchain already installed at $ZIG_BIN"
  else
    mkdir -p "$TC_DIR"
    download "https://ziglang.org/download/${ZIG_VERSION}/${tc_name}.${tc_ext}" \
             "$VENDOR/${tc_name}.${tc_ext}" "$tc_sha"
    say "extracting toolchain"
    rm -rf "$TC_DIR/$tc_name"
    unpack "$VENDOR/${tc_name}.${tc_ext}" "$TC_DIR"
    [[ -x "$ZIG_BIN" ]] || die "toolchain extract did not produce $ZIG_BIN"
  fi
fi

have_ver="$("$ZIG_BIN" version 2>/dev/null || true)"
[[ "$have_ver" == "$ZIG_VERSION" ]] || \
  die "oracle toolchain is zig '$have_ver', expected '$ZIG_VERSION'.
The corpus and the adjudicating stdlib must be the same release."

# --------------------------------------------------------------- 3. extract
command -v python3 >/dev/null 2>&1 || die "python3 is required to run the extractor"
say "extracting ZON documents from the pinned source"
DOCS="$VENDOR/docs.json"
python3 "$TOOLS/extract.py" "$SRC_DIR" > "$DOCS"

# ---------------------------------------------------------------- 4. oracle
say "building the oracle against zig ${ZIG_VERSION}"
mkdir -p "$VENDOR/oracle-build"
"$ZIG_BIN" build-exe "$TOOLS/oracle.zig" \
  -femit-bin="$VENDOR/oracle-build/oracle" \
  --cache-dir "$VENDOR/oracle-build/cache" \
  -OReleaseFast >/dev/null

say "adjudicating every document with the reference implementation"
python3 "$TOOLS/adjudicate.py" "$DOCS" "$VENDOR/oracle-build/oracle" \
  "$ZIG_UPSTREAM_REPO" "$ZIG_UPSTREAM_COMMIT" "$ZIG_VERSION" > "$CASES.part"
mv "$CASES.part" "$CASES"

# ------------------------------------------- 5. locally authored strictness probe
# Inputs are ours and tracked; the verdicts come from the same oracle, so the
# generated cases.json is not committed either.
say "adjudicating the strictness probe"
python3 "$TOOLS/probe.py" "$HERE/test/strictness/inputs.txt" \
  "$VENDOR/oracle-build/oracle" \
  "$ZIG_UPSTREAM_REPO" "$ZIG_UPSTREAM_COMMIT" "$ZIG_VERSION" \
  > "$HERE/test/strictness/cases.json.part"
mv "$HERE/test/strictness/cases.json.part" "$HERE/test/strictness/cases.json"

python3 - "$CASES" "$HERE/test/strictness/cases.json" <<'PY'
import json,sys
for label, path in (("corpus", sys.argv[1]), ("probe ", sys.argv[2])):
    c = json.load(open(path))["cases"]
    v = sum(1 for x in c if x["valid"]); n = len(c)
    print("[fetch-zigzon] %s ready: %d documents (%d valid / %d invalid) -> %s"
          % (label, n, v, n-v, path), file=sys.stderr)
PY
