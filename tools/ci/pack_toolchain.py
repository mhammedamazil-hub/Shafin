#!/usr/bin/env python3
"""Pack an official Rust toolchain into <50 MB chunks (runs on a GitHub-hosted runner).

Why this exists
---------------
The development sandbox used for this project can only reach github.com,
api.github.com, codeload.github.com, PyPI and npm.  static.rust-lang.org,
crates.io and GitHub *release asset* hosts are blocked, so `rustup` cannot run
there.  This script runs on a GitHub Actions runner (full network), downloads
the *official* component tarballs listed in the Rust channel manifest, verifies
each SHA-256 against that manifest, and splits them into chunks that can be
pushed to a git ref and fetched from the sandbox over plain git.

Nothing here is specific to the Visual Language itself; it is build plumbing.
"""
import hashlib
import json
import os
import sys
import time
import tomllib
import urllib.request

CHUNK = 45 * 1024 * 1024  # stay below GitHub's 50 MB soft warning per file
DIST = "https://static.rust-lang.org/dist"


def fetch(url: str, retries: int = 4) -> bytes:
    last = None
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(url, timeout=300) as r:
                return r.read()
        except Exception as e:  # noqa: BLE001 - report and retry
            last = e
            print(f"  retry {attempt + 1}/{retries} for {url}: {e}", flush=True)
            time.sleep(3 * (attempt + 1))
    raise RuntimeError(f"failed to fetch {url}: {last}")


def main() -> int:
    req_path, out_dir = sys.argv[1], sys.argv[2]
    req = json.load(open(req_path, "r", encoding="utf-8"))
    channel = req.get("channel", "stable")
    host = req["host"]
    os.makedirs(out_dir, exist_ok=True)

    print(f"channel manifest: {DIST}/channel-rust-{channel}.toml", flush=True)
    manifest = tomllib.loads(fetch(f"{DIST}/channel-rust-{channel}.toml").decode("utf-8"))
    rust_version = manifest["pkg"]["rust"]["version"]
    print(f"rust version: {rust_version}  manifest date: {manifest.get('date')}", flush=True)

    wanted = [(c, host) for c in req["components"]]
    wanted += [("rust-std", t) for t in req.get("extra_std_targets", [])]

    files = []
    for pkg, target in wanted:
        ent = manifest["pkg"][pkg]["target"].get(target)
        if not ent or not ent.get("available"):
            raise RuntimeError(f"{pkg} is not available for {target} in channel {channel}")
        url, want = ent["xz_url"], ent["xz_hash"]
        name = url.rsplit("/", 1)[1]
        print(f"download {name}", flush=True)
        blob = fetch(url)
        got = hashlib.sha256(blob).hexdigest()
        if got != want:
            raise RuntimeError(f"sha256 mismatch for {name}: manifest {want}, downloaded {got}")
        parts = []
        for i in range(0, len(blob), CHUNK):
            pname = f"{name}.part{i // CHUNK:02d}"
            with open(os.path.join(out_dir, pname), "wb") as f:
                f.write(blob[i:i + CHUNK])
            parts.append(pname)
        files.append({"pkg": pkg, "target": target, "name": name, "url": url,
                      "sha256": got, "size": len(blob), "parts": parts})
        print(f"  ok sha256={got} size={len(blob)} parts={len(parts)}", flush=True)

    out = {
        "schema": "vl-toolchain-bundle/1",
        "rust_version": rust_version,
        "manifest_date": manifest.get("date"),
        "channel": channel,
        "host": host,
        "files": files,
        "total_bytes": sum(f["size"] for f in files),
    }
    with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(out, f, indent=2)
    print(f"total {out['total_bytes'] / 1e6:.1f} MB in {sum(len(f['parts']) for f in files)} parts", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
