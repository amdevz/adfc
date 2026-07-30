#!/usr/bin/env bash
# End-to-end proof of the npm distribution path, without a registry.
#
# Builds a binary, wraps it in a dist-shaped archive, runs the packaging
# script, packs the two tarballs npm would publish, installs them into a
# throwaway prefix, and runs the result.
#
# Scope: the devShell ships no musl standard library, so the binary packaged
# here is the host build wearing the musl archive's name. That covers
# extraction, manifest generation, resolution and exec; the cross-compilation
# itself is dist's job on CI runners and is verified by the release pipeline.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="x86_64-unknown-linux-musl"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> building the binary"
cargo build --release --quiet

echo "==> staging a dist-shaped archive"
ARTIFACTS="$WORK/artifacts"
STAGE="$WORK/stage/adfc-$TARGET"
mkdir -p "$ARTIFACTS" "$STAGE"
cp "$ROOT/target/release/adfc" "$STAGE/adfc"
cp "$ROOT/LICENSE" "$ROOT/README.md" "$STAGE/"
tar -czf "$ARTIFACTS/adfc-$TARGET.tar.gz" -C "$WORK/stage" "adfc-$TARGET"

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
printf '{"releases":[{"app_name":"adfc","app_version":"%s"}]}\n' "$VERSION" > "$WORK/plan.json"

echo "==> generating npm packages (version $VERSION)"
node "$ROOT/scripts/build-npm-packages.js" \
  --plan "$WORK/plan.json" \
  --artifacts-dir "$ARTIFACTS" \
  --out-dir "$WORK/packages" \
  --only "$TARGET"

echo "==> packing tarballs"
# Scoped packages nest under their scope directory, and npm pack names the
# tarball with the scope flattened: @amdevz/adfc-linux-x64 -> amdevz-adfc-...
cd "$WORK/packages/@amdevz/adfc-linux-x64" && npm pack --quiet --pack-destination "$WORK" >/dev/null
cd "$WORK/packages/@amdevz/adfc" && npm pack --quiet --pack-destination "$WORK" >/dev/null

echo "==> installing into a throwaway prefix"
PREFIX="$WORK/prefix"
mkdir -p "$PREFIX"
cd "$PREFIX"
npm init -y --scope=e2e >/dev/null 2>&1
# --ignore-scripts is the load-bearing flag: it is the whole reason binaries
# ship inside the platform tarball rather than being fetched on install.
npm install --quiet --ignore-scripts --no-audit --no-fund \
  "$WORK/amdevz-adfc-linux-x64-$VERSION.tgz" "$WORK/amdevz-adfc-$VERSION.tgz" >/dev/null

BIN="$PREFIX/node_modules/.bin/adfc"
echo "==> running the installed binary"
test -x "$BIN" || { echo "FAIL: $BIN is not executable"; exit 1; }

ACTUAL_VERSION="$("$BIN" --version)"
echo "    $BIN --version -> $ACTUAL_VERSION"
test "$ACTUAL_VERSION" = "adfc $VERSION" || {
  echo "FAIL: expected 'adfc $VERSION', got '$ACTUAL_VERSION'"; exit 1
}

echo "==> converting through the installed binary"
OUT="$(printf '# Title\n\nSome **bold** text.\n' | "$BIN")"
echo "$OUT" | jq -e '.type == "doc" and .content[0].type == "heading"' >/dev/null || {
  echo "FAIL: unexpected ADF output: $OUT"; exit 1
}

echo "==> verifying it resolved via the platform package"
test -f "$PREFIX/node_modules/@amdevz/adfc-linux-x64/adfc" || {
  echo "FAIL: platform package binary missing"; exit 1
}

echo
echo "PASS: packed, installed with --ignore-scripts, and ran from node_modules"
