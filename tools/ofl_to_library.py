#!/usr/bin/env python3
"""Convert Open Fixture Library definitions into DMXpress library entries.

OFL (github.com/OpenLightingProject/open-fixture-library, MIT) describes a
fixture once and lists its DMX modes separately; DMXpress patches one *mode*
at a time, so every mode becomes its own library entry with the channel list
already flattened out in address order.

Pixel-matrix modes (LED bars and battens addressed per pixel) splice template
channel blocks into the mode's channel list; those are expanded here the way
OFL's own `Mode.channelKeys` does, so a 24-pixel bar's 74-channel mode comes
out as 74 real channels rather than being skipped.

Usually run through `tools/build_library.py`, which merges this with the
QLC+ converter. Standalone:

    curl -L -o ofl.tar.gz \\
        https://github.com/OpenLightingProject/open-fixture-library/archive/refs/heads/master.tar.gz
    tar xzf ofl.tar.gz --strip-components=1 open-fixture-library-master/fixtures
    python3 tools/ofl_to_library.py fixtures/ fixtures/library.json

The output matches `src/fixturedb.rs`: role names are the exact Rust `Role`
variants, and a channel's `bands` are the `showbuddy::Band` stepped ranges it
has labels for — see `plain()` for why most channels carry none.
"""

import gzip
import json
import os
import re
import sys
from collections import Counter

# OFL capability type -> DMXpress Role. Anything unlisted lands on Other,
# which still patches and is still addressable — it just doesn't colour-mix
# or show up as a Color/Position palette channel.
CAP_ROLE = {
    "Intensity": "Dimmer",
    "Pan": "Pan",
    "PanContinuous": "Pan",
    "Tilt": "Tilt",
    "TiltContinuous": "Tilt",
    "ShutterStrobe": "Strobe",
    "StrobeSpeed": "Strobe",
    "StrobeDuration": "Strobe",
    "Zoom": "Zoom",
    "Focus": "Focus",
    "Iris": "Iris",
    "IrisEffect": "Iris",
    "Frost": "Frost",
    "FrostEffect": "Frost",
    "Prism": "Prism",
    "PrismRotation": "Prism",
    "ColorPreset": "Color",
    "ColorTemperature": "Color",
    "PanTiltSpeed": "Speed",
    "EffectSpeed": "Speed",
    "Speed": "Speed",
}

# OFL ColorIntensity colours -> Role. Lime and Indigo have no DMXpress role of
# their own; folding them into the neighbouring primary keeps those emitters
# mixing into the visualiser instead of going dark.
COLOR_ROLE = {
    "Red": "Red",
    "Green": "Green",
    "Blue": "Blue",
    "White": "White",
    "Warm White": "White",
    "Cold White": "White",
    "Amber": "Amber",
    "UV": "Uv",
    "Cyan": "Cyan",
    "Magenta": "Magenta",
    "Yellow": "Yellow",
    "Lime": "Green",
    "Indigo": "Blue",
}

# Roles whose stepped ranges DMXpress actually reads back — colour/gobo wheel
# slot names drive the visualiser tint and the channel-grid tooltips, and a
# dimmer's labelled sub-range drives `Channel::dim_range`. Every other channel
# is emitted bandless, which keeps the library file small.
BANDED_ROLES = {"Color", "Gobo", "Dimmer"}

DEFAULT_PAN = 540.0
DEFAULT_TILT = 270.0
DEFAULT_BEAM = 25.0

# A DMX universe is 512 slots, so a mode wider than that can never be patched.
MAX_CHANNELS = 512


def deg(value, fallback):
    """Parse an OFL angle like '540deg' / '-270deg'."""
    if not isinstance(value, str):
        return fallback
    m = re.match(r"^(-?[\d.]+)\s*deg", value.strip())
    return float(m.group(1)) if m else fallback


def caps_of(ch):
    if "capabilities" in ch:
        return ch["capabilities"]
    if "capability" in ch:
        return [ch["capability"]]
    return []


def wheel_for(fixture, channel_key, cap):
    """The wheel a WheelSlot capability refers to (defaults to same-named)."""
    name = cap.get("wheel", channel_key)
    if isinstance(name, list):
        name = name[0] if name else channel_key
    return (fixture.get("wheels") or {}).get(name)


def cap_role(fixture, channel_key, cap):
    t = cap.get("type")
    if t == "ColorIntensity":
        return COLOR_ROLE.get(cap.get("color"), "Other")
    if t in ("WheelSlot", "WheelShake", "WheelRotation", "WheelSlotRotation"):
        wheel = wheel_for(fixture, channel_key, cap)
        slots = (wheel or {}).get("slots", [])
        kinds = Counter(s.get("type") for s in slots if s.get("type") not in (None, "Open", "Closed"))
        if kinds:
            top = kinds.most_common(1)[0][0]
            return {"Color": "Color", "Gobo": "Gobo", "Iris": "Iris",
                    "Frost": "Frost", "Prism": "Prism"}.get(top, "Gobo")
        return "Gobo"
    return CAP_ROLE.get(t, "Other")


def slot_label(fixture, channel_key, cap):
    """Human label for a stepped band: wheel slot name, shutter effect, etc."""
    t = cap.get("type")
    if t in ("WheelSlot", "WheelShake", "WheelSlotRotation"):
        wheel = wheel_for(fixture, channel_key, cap)
        n = cap.get("slotNumber")
        if wheel and isinstance(n, (int, float)):
            slots = wheel.get("slots", [])
            i = int(n) - 1
            if 0 <= i < len(slots):
                s = slots[i]
                return s.get("name") or s.get("type") or f"Slot {int(n)}"
        return f"Slot {n}" if n is not None else "Slot"
    if t == "ShutterStrobe":
        return cap.get("shutterEffect") or "Strobe"
    if t == "ColorPreset":
        return cap.get("comment") or "Color"
    if t == "Intensity":
        return cap.get("comment") or "Dimming"
    return cap.get("comment") or t or ""


def channel_role(fixture, wheel_key, ch):
    """The dominant role across a channel's capabilities."""
    roles = [cap_role(fixture, wheel_key, c) for c in caps_of(ch)]
    named = [r for r in roles if r != "Other"]
    return Counter(named).most_common(1)[0][0] if named else "Other"


def channel_entry(fixture, name, ch, fine_of=None, wheel_key=None):
    """One DMXpress channel: name, role and bands.

    `name` is what the mode calls the channel (for a matrix template channel
    that is the pixel-resolved name, e.g. "Red 3"); `wheel_key` is the key a
    WheelSlot capability resolves wheels against, which for a template channel
    is still the unresolved "Red $pixelKey".
    """
    wheel_key = wheel_key if wheel_key is not None else name
    caps = caps_of(ch)
    role = channel_role(fixture, wheel_key, ch)

    if fine_of is not None:
        # Only pan and tilt have somewhere to put the low byte; a "Dimmer
        # fine" would otherwise read as a second dimmer and fight the coarse
        # channel, so those stay unclassified.
        return plain(name, {"Pan": "PanFine", "Tilt": "TiltFine"}.get(fine_of, "Other"))

    if role in BANDED_ROLES and len(caps) > 1:
        bands = []
        for c in caps:
            lo, hi = (c.get("dmxRange") or [0, 255])[:2]
            bands.append(band("S", lo, hi, slot_label(fixture, wheel_key, c)))
        return {"name": name, "role": role, "bands": bands}

    return plain(name, role)


def plain(name, role):
    """A channel with no labelled sub-ranges.

    `bands` is left off entirely rather than holding one unlabelled 0-255
    band: every reader (`band_label`, `dim_range`, the encoder's stepped-value
    walk, `wheels::channel_slots`) ignores a full-range unlabelled band, so
    the two are equivalent — and with ~90% of the library's channels shaped
    like this, omitting it is a third of the file.
    """
    return {"name": name, "role": role}


def band(kind, lo, hi, label):
    """One labelled DMXpress band. OFL states 16-bit channel ranges at fine resolution
    (0-65535), but a band describes a single DMX slot, so those shift back
    down to 0-255."""
    lo, hi = int(lo), int(hi)
    while max(lo, hi) > 255:
        lo >>= 8
        hi >>= 8
    return {
        "kind": kind,
        "min": max(0, min(255, lo)),
        "max": max(0, min(255, hi)),
        "label": (label or "")[:28],
    }


def geometry(fixture, mode):
    """Pan/tilt travel and beam angle, with mode-level physical overrides."""
    phys = dict(fixture.get("physical") or {})
    phys.update(mode.get("physical") or {})
    lens = phys.get("lens") or {}
    beam = DEFAULT_BEAM
    if isinstance(lens.get("degreesMinMax"), list) and lens["degreesMinMax"]:
        beam = float(max(lens["degreesMinMax"]))
    focus = phys.get("focus") or {}
    pan = float(focus.get("panMax") or 0) or None
    tilt = float(focus.get("tiltMax") or 0) or None
    # Most files express travel on the Pan/Tilt capability instead. Matrix
    # fixtures keep their per-pixel pan/tilt in templateChannels, so scan both.
    sources = list((fixture.get("availableChannels") or {}).values())
    sources += list((fixture.get("templateChannels") or {}).values())
    for ch in sources:
        for c in caps_of(ch):
            if c.get("type") == "Pan" and pan is None:
                pan = abs(deg(c.get("angleEnd"), DEFAULT_PAN) - deg(c.get("angleStart"), 0.0))
            if c.get("type") == "Tilt" and tilt is None:
                tilt = abs(deg(c.get("angleEnd"), DEFAULT_TILT) - deg(c.get("angleStart"), 0.0))
    return (pan or DEFAULT_PAN, tilt or DEFAULT_TILT, beam)


# --------------------------------------------------------------------------
# Pixel matrices
#
# Mirrors OFL's `lib/model/Matrix.js` closely enough that channel order comes
# out identical: pixel keys are auto-named from `pixelCount` when the file
# doesn't spell them out, sorted alphanumerically for `eachPixelABC`, and by
# axis position for `eachPixelXYZ` and friends.
# --------------------------------------------------------------------------


def natural_key(text):
    """Sort key matching JS `localeCompare(…, {numeric: true})`: digit runs
    compare as numbers and sort before letters ("1" < "2" < "10" < "alice")."""
    return [
        (0, int(part), "") if part.isdigit() else (1, 0, part.lower())
        for part in re.split(r"(\d+)", str(text))
        if part != ""
    ]


class Matrix:
    """The pixel layout of one fixture."""

    def __init__(self, js):
        self.js = js or {}
        self.structure = self._structure()
        self.count = self._count()
        self.positions = self._positions()
        # Alphanumeric order is both `eachPixelABC` and the stable base the
        # axis orders re-sort, exactly as OFL does it.
        self.keys = sorted(self.positions, key=natural_key)

    def _count(self):
        if "pixelCount" in self.js:
            return list(self.js["pixelCount"])
        xyz = [1, 1, len(self.structure)]
        for y_items in self.structure:
            xyz[1] = max(xyz[1], len(y_items))
            for x_items in y_items:
                xyz[0] = max(xyz[0], len(x_items))
        return xyz

    def _structure(self):
        if "pixelKeys" in self.js:
            return self.js["pixelKeys"]
        cx, cy, cz = self.js.get("pixelCount", [1, 1, 1])
        axes = [n > 1 for n in (cx, cy, cz)]
        return [
            [[self._default_key(x, y, z, axes) for x in range(1, cx + 1)]
             for y in range(1, cy + 1)]
            for z in range(1, cz + 1)
        ]

    @staticmethod
    def _default_key(x, y, z, axes):
        """OFL's auto-naming: "3" on a strip, "(2, 5)" on a grid."""
        has_x, has_y, _ = axes
        n = sum(axes)
        if n <= 1:
            return str(max(x, y, z))
        if n == 2:
            return f"({x if has_x else y}, {y if has_y else z})"
        return f"({x}, {y}, {z})"

    def _positions(self):
        """pixel key -> 1-based (x, y, z)."""
        out = {}
        for z, y_items in enumerate(self.structure):
            for y, x_items in enumerate(y_items):
                for x, key in enumerate(x_items):
                    if key is not None:
                        out[key] = (x + 1, y + 1, z + 1)
        return out

    def keys_by_axes(self, order):
        """`eachPixelXYZ` and friends: the named axis varies fastest."""
        index = {"X": 0, "Y": 1, "Z": 2}
        try:
            first, second, third = (index[a] for a in order[:3])
        except (KeyError, ValueError):
            return list(self.keys)
        return sorted(
            self.keys,
            key=lambda k: (
                self.positions[k][third],
                self.positions[k][second],
                self.positions[k][first],
            ),
        )

    def group_keys(self):
        """Pixel group names, in the order the file declares them. Membership
        never matters here — only the name a template channel resolves to."""
        return list((self.js.get("pixelGroups") or {}).keys())

    def repeat_keys(self, repeat_for):
        if isinstance(repeat_for, list):
            return repeat_for
        if repeat_for == "eachPixelGroup":
            return self.group_keys()
        if repeat_for == "eachPixelABC":
            return list(self.keys)
        if isinstance(repeat_for, str) and repeat_for.startswith("eachPixel"):
            return self.keys_by_axes(repeat_for[len("eachPixel"):])
        return []


def template_lookup(fixture, matrix):
    """Resolved channel name -> (template key, channel, fine-parent role).

    Covers every pixel key and pixel group name, because a mode can reference
    a resolved template channel by name ("Strobe Master") as well as through
    an insert block.
    """
    templates = fixture.get("templateChannels") or {}
    if not templates:
        return {}
    names = list(matrix.keys) + matrix.group_keys()
    out = {}
    for key, ch in templates.items():
        role = channel_role(fixture, key, ch)
        for pixel in names:
            out[key.replace("$pixelKey", str(pixel))] = (key, ch, None)
            for alias in ch.get("fineChannelAliases") or []:
                out[alias.replace("$pixelKey", str(pixel))] = (key, ch, role)
    return out


def flatten_slots(slots, matrix):
    """The mode's channel list with matrix insert blocks expanded in place."""
    out = []
    for slot in slots:
        if not isinstance(slot, dict):
            out.append(slot)
            continue
        if slot.get("insert") != "matrixChannels":
            # An insert kind this converter doesn't know would silently shift
            # every later address, so refuse the mode instead.
            return None
        pixels = matrix.repeat_keys(slot.get("repeatFor"))
        templates = slot.get("templateChannels") or []
        if not pixels or not templates:
            return None
        if slot.get("channelOrder") == "perChannel":
            pairs = ((t, p) for t in templates for p in pixels)
        else:  # perPixel, OFL's default and the only order in use today
            pairs = ((t, p) for p in pixels for t in templates)
        for template, pixel in pairs:
            out.append(None if template is None
                       else template.replace("$pixelKey", str(pixel)))
    return out


def convert(root):
    """Every patchable mode in an OFL `fixtures/` tree, as library entries."""
    manufacturers = json.load(open(os.path.join(root, "manufacturers.json"), encoding="utf-8"))
    entries, skipped_modes, skipped_fixtures = [], 0, 0

    for man_key in sorted(os.listdir(root)):
        man_dir = os.path.join(root, man_key)
        if not os.path.isdir(man_dir):
            continue
        man_name = (manufacturers.get(man_key) or {}).get("name", man_key)

        for fname in sorted(os.listdir(man_dir)):
            if not fname.endswith(".json"):
                continue
            with open(os.path.join(man_dir, fname), encoding="utf-8") as fh:
                fixture = json.load(fh)
            fix_key = fname[:-5]
            available = fixture.get("availableChannels") or {}
            matrix = Matrix(fixture.get("matrix"))
            templates = template_lookup(fixture, matrix)

            # alias -> (parent channel key, parent role source)
            fine = {}
            for key, ch in available.items():
                for alias in ch.get("fineChannelAliases") or []:
                    fine[alias] = (key, channel_role(fixture, key, ch))
            # Switching channels resolve to whichever channel they can become;
            # take the first, so at least the role is usually right.
            switching = {}
            for key, ch in available.items():
                for c in caps_of(ch):
                    for alias, target in (c.get("switchChannels") or {}).items():
                        switching.setdefault(alias, target)

            emitted_any = False
            for mode in fixture.get("modes") or []:
                slots = flatten_slots(mode.get("channels") or [], matrix)
                if slots is None or not slots or len(slots) > MAX_CHANNELS:
                    skipped_modes += 1
                    continue

                channels = []
                for i, slot in enumerate(slots):
                    if slot is None:
                        channels.append(
                            plain(f"Unused {i + 1}", "Other")
                        )
                    elif slot in available:
                        channels.append(channel_entry(fixture, slot, available[slot]))
                    elif slot in fine:
                        parent, parent_role = fine[slot]
                        channels.append(
                            channel_entry(fixture, slot, available[parent], fine_of=parent_role)
                        )
                    elif slot in templates:
                        key, ch, fine_of = templates[slot]
                        channels.append(
                            channel_entry(fixture, slot, ch, fine_of=fine_of, wheel_key=key)
                        )
                    elif slot in switching and switching[slot] in available:
                        channels.append(
                            channel_entry(fixture, slot, available[switching[slot]])
                        )
                    else:
                        # Never drop a slot: the address span has to stay exact.
                        channels.append(
                            plain(slot, "Other")
                        )

                pan, tilt, beam = geometry(fixture, mode)
                entries.append({
                    "id": f"{man_key}/{fix_key}/{mode.get('shortName') or mode['name']}",
                    "manufacturer": man_name,
                    "model": fixture.get("name", fix_key),
                    "mode": mode.get("name", ""),
                    "category": (fixture.get("categories") or ["Other"])[0],
                    "pan_range": round(pan, 1),
                    "tilt_range": round(tilt, 1),
                    "beam_width": round(beam, 1),
                    "channels": channels,
                })
                emitted_any = True

            if not emitted_any:
                skipped_fixtures += 1

    print(
        f"OFL: {len(entries)} modes, skipped {skipped_modes} unexpandable modes "
        f"and {skipped_fixtures} fixtures with no usable mode",
        file=sys.stderr,
    )
    return entries


def write(entries, out_path, source):
    """Write the library, gzipped when `out_path` ends in `.gz`.

    Sorted by maker, then model, then channel count, so the patch browser
    lists a fixture's modes together and in the order you'd pick between
    them — 9-channel before 15-channel before 30-channel.
    """
    entries.sort(key=lambda e: (e["manufacturer"].lower(), e["model"].lower(), len(e["channels"])))
    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    payload = json.dumps({"version": 1, "source": source, "fixtures": entries},
                         separators=(",", ":")).encode("utf-8")
    if out_path.endswith(".gz"):
        # mtime=0 so rebuilding unchanged data produces an identical file
        # rather than a spurious diff.
        with gzip.GzipFile(out_path, "wb", compresslevel=9, mtime=0) as fh:
            fh.write(payload)
    else:
        with open(out_path, "wb") as fh:
            fh.write(payload)
    mans = len({e["manufacturer"] for e in entries})
    models = len({(e["manufacturer"], e["model"]) for e in entries})
    size = os.path.getsize(out_path) / 1e6
    print(f"{len(entries)} modes / {models} models / {mans} manufacturers "
          f"-> {out_path} ({size:.1f} MB, {len(payload) / 1e6:.1f} MB raw)")


SOURCE = "Open Fixture Library (MIT) — github.com/OpenLightingProject/open-fixture-library"


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    write(convert(sys.argv[1]), sys.argv[2], SOURCE)
