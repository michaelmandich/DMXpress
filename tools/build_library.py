#!/usr/bin/env python3
"""Build `fixtures/library.json.gz` from every fixture source DMXpress uses.

No upstream is complete on its own:

  extra.json  entries written by hand, because a rebuild must never drop them
  OFL         ~640 models, described more precisely than anyone else
  QLC+        ~1750 models, and far more *modes* per model — the 6/7/8/13-
              channel variants of one par, the 9- and 15-channel modes of one
              moving head
  GDTF        the manufacturers' own data, and the only source that reliably
              carries the current professional ranges (Robe MegaPointe and
              BMFL, Ayrton Khamsin, High End SolaFrame, the Vari-Lite VLs)

They merge in that order, and a mode is dropped when an earlier source already
covers the same manufacturer and model at the same channel count — which is
what the patch window actually asks you to choose between. Order matters for
more than quality: a patched show stores the library id of the mode it was
patched with, so a source may only ever *add* ids, never displace them.

    curl -L -o ofl.tar.gz \\
        https://github.com/OpenLightingProject/open-fixture-library/archive/refs/heads/master.tar.gz
    curl -L -o qlc.tar.gz \\
        https://github.com/mcallegari/qlcplus/archive/refs/heads/master.tar.gz
    mkdir -p build/ofl build/qlc
    tar xzf ofl.tar.gz --strip-components=1 -C build/ofl open-fixture-library-master/fixtures
    tar xzf qlc.tar.gz --strip-components=1 -C build/qlc qlcplus-master/resources/fixtures
    python3 tools/gdtf_fetch.py build/gdtf          # needs a free GDTF Share login
    python3 tools/build_library.py build/ofl/fixtures build/qlc/resources/fixtures \\
        fixtures/library.json.gz --gdtf build/gdtf
    cargo test fixturedb

`--gdtf` is optional; leave it off and the library is built from the other
three. Pass an output path without the `.gz` to get the same data
uncompressed, which is a great deal easier to read while checking a
conversion.
"""

import argparse
import json
import os
import re
import sys

import ofl_to_library
import qlc_to_library

# Entries written by hand rather than converted — a fixture neither upstream
# carries, or one whose generated channel list needed correcting.
EXTRA_FILE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                          "fixtures", "extra.json")

# Manufacturers whose names genuinely differ between sources, rather than
# merely differing in case or punctuation — those are handled automatically by
# `preferred_spellings`. Keyed by the lowercase-alphanumeric form.
CANONICAL = {
    "blizzard": "Blizzard Lighting",
    "blizzardlighting": "Blizzard Lighting",
    "megalite": "Mega Lite",
    "prolights": "Prolights",
}

# Manufacturer spellings to reject when something better is available: all of
# one case tells you nothing, and "ELATION" next to "Elation" would otherwise
# split one maker across two rows of the Patch window's dropdown.
def spelling_score(name):
    """Higher is a better display name for a manufacturer."""
    letters = [c for c in name if c.isalpha()]
    if not letters:
        return 0
    mixed = not (all(c.isupper() for c in letters) or all(c.islower() for c in letters))
    # A properly cased name beats a shouted or whispered one; among equals,
    # prefer the one with real spacing ("Clay Paky" over "ClayPaky").
    return (2 if mixed else 0) + (1 if " " in name.strip() else 0)


def preferred_spellings(entries):
    """One display spelling per manufacturer, chosen from what the sources
    actually wrote. GDTF shouts ("ELATION"), OFL sometimes whispers
    ("cameo"), and QLC+ hyphenates; this picks the readable one without
    needing a hand-written entry for every maker."""
    best = {}
    for entry in entries:
        name = entry["manufacturer"]
        key = norm(name)
        current = best.get(key)
        if current is None or spelling_score(name) > spelling_score(current):
            best[key] = name
    return best

# Brands one source splits by product line and the other doesn't: OFL files
# Chauvet under "Chauvet DJ" and "Chauvet Professional", QLC+ lumps both into
# "Chauvet". Those stay separate in the dropdown — nobody wants 490 Chauvet
# entries in one list, and searching "chauvet" still finds all of them — but
# they share a key here so the same fixture isn't listed twice.
DEDUPE_GROUP = {
    "chauvetdj": "chauvet",
    "chauvetprofessional": "chauvet",
    "flashprofessional": "flash",
    "flashbutrym": "flash",
}

SOURCE = (
    "Open Fixture Library (MIT) — github.com/OpenLightingProject/open-fixture-library; "
    "QLC+ fixture definitions (Apache-2.0) — github.com/mcallegari/qlcplus; "
    "GDTF Share — gdtf-share.com; plus DMXpress's own fixtures/extra.json"
)


def norm(value):
    return re.sub(r"[^a-z0-9]", "", str(value).lower())


def canonical_manufacturer(name):
    return CANONICAL.get(norm(name), name)


def dedupe_key(entry):
    """What makes two entries the same patchable thing: one model of one
    maker, at one channel count."""
    maker = norm(entry["manufacturer"])
    return (DEDUPE_GROUP.get(maker, maker), norm(entry["model"]), len(entry["channels"]))


def hand_written():
    """`fixtures/extra.json`, or nothing if it isn't there."""
    if not os.path.exists(EXTRA_FILE):
        return []
    with open(EXTRA_FILE, encoding="utf-8") as fh:
        entries = json.load(fh).get("fixtures") or []
    print(f"extra.json: {len(entries)} hand-written modes", file=sys.stderr)
    return entries


def merge(sources):
    """The `(label, entries)` sources in priority order, with each entry that
    an earlier source already covers left out.

    Only ever compares *across* sources. One fixture routinely has several
    modes of the same width — a 32-channel "16-bit" mode next to a 32-channel
    "extended" one — and those are genuinely different things to patch, so a
    source is never deduped against itself.
    """
    out, seen = [], set()
    for label, entries in sources:
        kept = [e for e in entries if dedupe_key(e) not in seen]
        out.extend(kept)
        seen.update(dedupe_key(e) for e in kept)
        print(f"  {label}: added {len(kept)} of {len(entries)} "
              f"({len(entries) - len(kept)} already covered)", file=sys.stderr)
    return out


def gdtf_entries(gdtf_dir):
    """GDTF conversions, or nothing when no directory was given.

    Imported lazily so the other three sources still build on a machine with
    no `pygdtf` installed.
    """
    if not gdtf_dir:
        return []
    if not os.path.isdir(gdtf_dir):
        sys.exit(f"--gdtf {gdtf_dir}: no such directory. Run tools/gdtf_fetch.py first.")
    import gdtf_to_library
    return gdtf_to_library.convert(gdtf_dir)


def build(ofl_dir, qlc_dir, out_path, gdtf_dir=None):
    sources = [
        ("extra", hand_written()),
        ("ofl", ofl_to_library.convert(ofl_dir)),
        ("qlc+", qlc_to_library.convert(qlc_dir)),
        ("gdtf", gdtf_entries(gdtf_dir)),
    ]
    everything = [e for _, entries in sources for e in entries]
    for entry in everything:
        entry["manufacturer"] = canonical_manufacturer(entry["manufacturer"])
    spelling = preferred_spellings(everything)
    for entry in everything:
        entry["manufacturer"] = spelling.get(norm(entry["manufacturer"]), entry["manufacturer"])
    ofl_to_library.write(merge(sources), out_path, SOURCE)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ofl_dir", help="an OFL checkout's fixtures/ directory")
    parser.add_argument("qlc_dir", help="a QLC+ checkout's resources/fixtures/ directory")
    parser.add_argument("out_path", help="fixtures/library.json.gz")
    parser.add_argument("--gdtf", dest="gdtf_dir",
                        help="directory of .gdtf files from tools/gdtf_fetch.py")
    args = parser.parse_args()
    build(args.ofl_dir, args.qlc_dir, args.out_path, args.gdtf_dir)
