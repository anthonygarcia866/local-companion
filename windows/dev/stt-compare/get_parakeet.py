"""Fetches NVIDIA's Parakeet TDT 0.6B v3 (int8, sherpa-onnx export) for compare.py --parakeet.

Dev-only; never ships, and Glim never downloads it. The four files, their
pinned Hugging Face revision, sizes and SHA-256 are upstream Coucou's
(windows/src-tauri/src/speech/mod.rs, `PARAKEET`), so the comparison runs
the exact model upstream ships on Windows. About 670 MB; each file is
checked against its SHA-256 and size, and a file already there with the
right hash is not fetched again. Licence: the model is CC-BY-4.0 (NVIDIA),
sherpa-onnx is Apache-2.0.

Usage: python -I get_parakeet.py <model dir>      (a scratch dir, never the repo)
       python -I get_parakeet.py --check <model dir>   (hashes only, no network)
"""

import hashlib
import os
import sys
import urllib.request

REPO = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78"

# (name, size in bytes, SHA-256), from upstream's speech/mod.rs.
FILES = [
    ("encoder.int8.onnx", 652_184_281, "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247"),
    ("decoder.int8.onnx", 11_845_275, "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e"),
    ("joiner.int8.onnx", 6_355_277, "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3"),
    ("tokens.txt", 93_939, "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d"),
]


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def good(path: str, size: int, digest: str) -> bool:
    return os.path.isfile(path) and os.path.getsize(path) == size and sha256(path) == digest


def fetch(url: str, dest: str):
    """Downloads to dest + '.part', then renames: a cut download never looks done."""
    part = dest + ".part"
    with urllib.request.urlopen(url, timeout=60) as r, open(part, "wb") as f:
        while block := r.read(1 << 20):
            f.write(block)
    os.replace(part, dest)


def main():
    args = sys.argv[1:]
    check_only = bool(args) and args[0] == "--check"
    if check_only:
        args = args[1:]
    if len(args) != 1:
        sys.exit(__doc__)
    out = args[0]
    if os.path.abspath(out).lower().startswith(os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..")).lower() + os.sep):
        sys.exit("refusing to write the model inside the repo: use a scratch dir")
    os.makedirs(out, exist_ok=True)
    bad = []
    for name, size, digest in FILES:
        path = os.path.join(out, name)
        if good(path, size, digest):
            print(f"ok    {name}")
            continue
        if check_only:
            bad.append(name)
            print(f"BAD   {name}")
            continue
        print(f"fetch {name} ({size / 1e6:.0f} MB)", flush=True)
        fetch(f"{REPO}/{name}", path)
        if not good(path, size, digest):
            os.remove(path)
            sys.exit(f"{name}: size or SHA-256 doesn't match the pinned one; removed")
        print(f"ok    {name}")
    if bad:
        sys.exit(f"missing or wrong: {', '.join(bad)}")


if __name__ == "__main__":
    main()
