"""Turn the rendered screenshots into the WebP the website serves.

    cargo test site_shots -- --test-threads=1     # renders PNGs to target/site-shots/
    python tools/site_shots.py                    # converts them into dist-netlify/images/

`src/site_shots.rs` renders PNGs because that is all the `image` crate is built
with here. The site serves WebP because the same pictures as PNG are about five
times the bytes: 25 MB of screenshots made the home page a 23 MB download, which
is a page nobody waits for. At quality 90 they are indistinguishable from the
PNGs at 1:1 even on the smallest UI text, and the whole set fits in under 5 MB.

Quality is deliberately high. These are screenshots of a dark UI full of thin
9-11px type, which is the worst case for a lossy codec; dropping to 80 saves
little and starts to ring around the text.

The favicon stays a PNG, because a favicon is the one place WebP is still worth
avoiding.
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:  # pragma: no cover - a helper script, not part of the build
    sys.exit("Pillow is needed: python -m pip install Pillow")

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "target" / "site-shots"
DST = ROOT / "dist-netlify" / "images"
QUALITY = 90


def main() -> int:
    if not SRC.is_dir():
        sys.exit(f"no renders in {SRC} — run: cargo test site_shots -- --test-threads=1")

    DST.mkdir(parents=True, exist_ok=True)
    shots = sorted(SRC.glob("*.png"))
    if not shots:
        sys.exit(f"{SRC} has no PNGs in it")

    before = after = 0
    for png in shots:
        out = DST / (png.stem + ".webp")
        with Image.open(png) as im:
            # The renders are opaque; dropping the alpha channel costs nothing
            # and saves a quarter of the bytes.
            im.convert("RGB").save(out, "WEBP", quality=QUALITY, method=6)
        before += png.stat().st_size
        after += out.stat().st_size
        print(f"  {png.name:34} {png.stat().st_size // 1024:5} KB -> {out.stat().st_size // 1024:5} KB")

    print(
        f"\n{len(shots)} shots: {before / 1048576:.1f} MB of PNG -> "
        f"{after / 1048576:.1f} MB of WebP ({100 * after / before:.0f}%)"
    )
    print(f"written to {DST.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
