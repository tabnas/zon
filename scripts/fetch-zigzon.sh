#!/usr/bin/env bash
#
# fetch-zigzon.sh — build the ZON conformance corpora from the ZIG REFERENCE
# IMPLEMENTATION. Opt-in and idempotent; nothing it writes is committed.
#
#     bash scripts/fetch-zigzon.sh
#
# What it does:
#
#   1. downloads the pinned zig SOURCE tarball and the pinned prebuilt zig
#      TOOLCHAIN for this host, into test/zigzon/vendor/ (~80 MB, cached)
#   2. builds test/zigzon/tools/oracle.zig with that toolchain — a batch judge
#      wrapping the compiler's own std.zig.Ast + std.zig.ZonGen
#   3. harvests every ZON document in the zig tree (test/zigzon/tools/harvest.py)
#      and asks the oracle for each verdict + value
#        -> test/zigzon/cases.json
#   4. runs the locally authored probes in test/strictness/inputs.txt through
#      the same oracle
#        -> test/strictness/cases.json
#
# Then `npm test` (ts/) and `go test ./...` (go/) pick the corpora up; without
# them the conformance suites skip. This repo never decides what ZON means:
# every verdict in both corpora comes from the pinned compiler.
#
# Requires: bash, curl, tar, python3. Network access on first run only.

set -euo pipefail

ZIG_VERSION="0.16.0"
# The ziglang/zig commit the release was cut from, recorded in the corpora so
# a result can always be traced back to an exact tree.
ZIG_COMMIT="24fdd5b7a4c1c8b5deb5b56756b9dbc8e08c86a8"
ZIG_REPO="https://codeberg.org/ziglang/zig"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR="$REPO_ROOT/test/zigzon/vendor"
TOOLS="$REPO_ROOT/test/zigzon/tools"
SRC_DIR="$VENDOR/zig-$ZIG_VERSION"
TOOLCHAIN_DIR="$VENDOR/toolchain"
BUILD_DIR="$VENDOR/oracle-build"

mkdir -p "$VENDOR" "$BUILD_DIR"

# --- host triple for the prebuilt toolchain --------------------------------
case "$(uname -s)" in
  Linux)  os="linux" ;;
  Darwin) os="macos" ;;
  *) echo "fetch-zigzon: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *) echo "fetch-zigzon: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac
TRIPLE="$arch-$os"
TOOLCHAIN="$TOOLCHAIN_DIR/zig-$TRIPLE-$ZIG_VERSION"

fetch() { # url dest
  [ -f "$2" ] && return 0
  echo "fetch-zigzon: downloading $1"
  curl --fail --location --progress-bar --output "$2.part" "$1"
  mv "$2.part" "$2"
}

# --- 1. the prebuilt toolchain ---------------------------------------------
TOOLCHAIN_TAR="$VENDOR/zig-$TRIPLE-$ZIG_VERSION.tar.xz"
if [ ! -x "$TOOLCHAIN/zig" ]; then
  fetch "https://ziglang.org/download/$ZIG_VERSION/zig-$TRIPLE-$ZIG_VERSION.tar.xz" \
    "$TOOLCHAIN_TAR"
  mkdir -p "$TOOLCHAIN_DIR"
  tar -xJf "$TOOLCHAIN_TAR" -C "$TOOLCHAIN_DIR"
fi
ZIG="$TOOLCHAIN/zig"
echo "fetch-zigzon: toolchain $("$ZIG" version)"

# --- 2. the source tree (only the parts that hold ZON) ---------------------
SRC_TAR="$VENDOR/zig-$ZIG_VERSION.tar.xz"
if [ ! -f "$SRC_DIR/.fetched" ]; then
  fetch "https://ziglang.org/download/$ZIG_VERSION/zig-$ZIG_VERSION.tar.xz" "$SRC_TAR"
  rm -rf "$SRC_DIR"
  mkdir -p "$SRC_DIR"
  # Everything that can contain a ZON document: the manifests, the codegen
  # tables, the behaviour/compile-error fixtures, and the std zon parser
  # (whose tests embed ~150 more snippets).
  tar -xJf "$SRC_TAR" -C "$SRC_DIR" --strip-components=1 --wildcards \
    '*/build.zig.zon' \
    '*/lib/init/*' \
    '*/lib/std/zon/*' \
    '*/src/codegen/*' \
    '*/test/behavior/zon/*' \
    '*/test/cases/compile_errors/zon/*'
  touch "$SRC_DIR/.fetched"
fi

# --- 3. the oracle ---------------------------------------------------------
ORACLE="$BUILD_DIR/oracle"
echo "fetch-zigzon: building the oracle"
"$ZIG" build-exe "$TOOLS/oracle.zig" \
  -O ReleaseSafe \
  --cache-dir "$BUILD_DIR/cache" \
  -femit-bin="$ORACLE"

# --- 4. the corpora --------------------------------------------------------
echo "fetch-zigzon: harvesting ZON documents from the zig tree"
python3 "$TOOLS/harvest.py" "$SRC_DIR" > "$BUILD_DIR/harvest.json"

echo "fetch-zigzon: judging with zig $ZIG_VERSION"
ZIG_VERSION="$ZIG_VERSION" ZIG_COMMIT="$ZIG_COMMIT" ZIG_REPO="$ZIG_REPO" \
python3 "$TOOLS/judge.py" \
  --oracle "$ORACLE" \
  --harvest "$BUILD_DIR/harvest.json" \
  --inputs "$REPO_ROOT/test/strictness/inputs.txt" \
  --zigzon-out "$REPO_ROOT/test/zigzon/cases.json" \
  --strictness-out "$REPO_ROOT/test/strictness/cases.json"

echo "fetch-zigzon: done"
echo "  test/zigzon/cases.json"
echo "  test/strictness/cases.json"
