#!/usr/bin/env bash
# Install the Rust toolchain in a network-restricted sandbox.
#
# Normal machines do NOT need this: install Rust with rustup (https://rustup.rs)
# and run `cargo build --release`. The project has zero third-party crates.
#
# This script is for sandboxes that can reach github.com but not
# static.rust-lang.org. It fetches the bundle that the workflow
# .github/workflows/vl-bootstrap-toolchain.yml published on the hidden git ref
# refs/vl-bootstrap/toolchain, re-verifies every SHA-256 recorded in the bundle
# manifest (taken from the official Rust channel manifest on the runner) and
# installs into $VL_TOOLCHAIN_DIR (default: ~/.vl-toolchain), outside the repo.
set -euo pipefail

REPO_URL="${VL_REPO_URL:-$(git -C "$(dirname "$0")/../.." remote get-url origin)}"
ROOT="${VL_TOOLCHAIN_DIR:-$HOME/.vl-toolchain}"
BUNDLE="$ROOT/bundle"
PREFIX="$ROOT/rust"

mkdir -p "$BUNDLE"
cd "$BUNDLE"
[ -d .git ] || git init -q .
echo "[bootstrap] fetching refs/vl-bootstrap/toolchain from $REPO_URL"
git fetch -q --depth=1 "$REPO_URL" refs/vl-bootstrap/toolchain
git checkout -q -f FETCH_HEAD

python3 - "$BUNDLE" <<'PY'
import hashlib, json, os, sys
b = sys.argv[1]
m = json.load(open(os.path.join(b, "manifest.json")))
print(f"[bootstrap] rust {m['rust_version']} ({m['total_bytes']/1e6:.1f} MB)")
for f in m["files"]:
    h = hashlib.sha256()
    out = os.path.join(b, f["name"])
    with open(out, "wb") as w:
        for p in f["parts"]:
            data = open(os.path.join(b, p), "rb").read()
            h.update(data); w.write(data)
    if h.hexdigest() != f["sha256"]:
        sys.exit(f"sha256 mismatch for {f['name']}")
    print(f"[bootstrap] verified {f['name']}")
PY

rm -rf "$PREFIX" "$ROOT/unpack"
mkdir -p "$PREFIX" "$ROOT/unpack"
for t in "$BUNDLE"/*.tar.xz; do
  name="$(basename "$t" .tar.xz)"
  tar -xJf "$t" -C "$ROOT/unpack"
  ( cd "$ROOT/unpack/$name" && ./install.sh --prefix="$PREFIX" --disable-ldconfig >/dev/null )
  rm -rf "$ROOT/unpack/$name" "$t"
  echo "[bootstrap] installed $name"
done
rm -rf "$ROOT/unpack"

cat > "$ROOT/env.sh" <<ENV
# source this file:  . $ROOT/env.sh
export PATH="$PREFIX/bin:\$PATH"
export CARGO_HOME="$ROOT/cargo-home"
export CARGO_NET_OFFLINE=true
ENV
echo "[bootstrap] done. Run:  . $ROOT/env.sh"
"$PREFIX/bin/rustc" --version
"$PREFIX/bin/cargo" --version

# A restored sandbox snapshot has been seen to re-encode the non-UTF-8 bytes of binary test
# data (2026-11-14: crates/vl-image/tests/data/ref_dynamic.bin grew from 29972 to 32362
# bytes). Nothing is changed here; this only tells you if it has happened again.
if command -v git >/dev/null 2>&1; then
  changed=$(git -C "$(dirname "$0")/../.." status --short -- crates/vl-image/tests/data 2>/dev/null || true)
  if [ -n "$changed" ]; then
    echo "[bootstrap] WARNING: binary test data differs from git:"
    echo "$changed"
    echo "[bootstrap] if you did not change it, restore it with:  git checkout -- crates/vl-image/tests/data"
  fi
fi
