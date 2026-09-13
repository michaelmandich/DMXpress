#!/usr/bin/env python3
"""Convert Open Fixture Library definitions into DMXpress's fixture library.

OFL (github.com/OpenLightingProject/open-fixture-library, MIT) describes a
fixture once and lists its DMX modes separately; DMXpress patches one *mode*
at a time, so every mode becomes its own library entry with the channel list
already flattened out in address order.

Usage:
    curl -L -o ofl.tar.gz \\
        https://github.com/OpenLightingProject/open-fixture-library/archive/refs/heads/master.tar.gz
    tar xzf ofl.tar.gz --strip-components=1 open-fixture-library-master/fixtures
    python3 tools/ofl_to_library.py fixtures/ fixtures/library.json

The output matches `src/fixturedb.rs`: role names are the exact Rust `Role`
variants, and band `kind` is 'D' (dimmer), 'V' (continuous) or 'S' (stepped),
matching `showbuddy::Band`.
"""

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
# collapses to one full-range band, which keeps the library file small.
BANDED_ROLES = {"Color", "Gobo", "Dimmer"}

DEFAULT_PAN = 540.0
DEFAULT_TILT = 270.0
DEFAULT_BEAM = 25.0


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


def channel_entry(fixture, key, ch, fine_of=None):
    """One DMXpress channel: name, role and bands."""
    caps = caps_of(ch)
    roles = [cap_role(fixture, key, c) for c in caps]
    named = [r for r in roles if r != "Other"]
    role = Counter(named).most_common(1)[0][0] if named else "Other"

    if fine_of is not None:
        # Only pan and tilt have somewhere to put the low byte; a "Dimmer
        # fine" would otherwise read as a second dimmer and fight the coarse
        # channel, so those stay unclassified.
        role = {"Pan": "PanFine", "Tilt": "TiltFine"}.get(fine_of, "Other")
        return {"name": key, "role": role, "bands": [band("V", 0, 255, "")]}

    if role in BANDED_ROLES and len(caps) > 1:
        bands = []
        for c in caps:
            lo, hi = (c.get("dmxRange") or [0, 255])[:2]
            bands.append(band("S", lo, hi, slot_label(fixture, key, c)))
        return {"name": key, "role": role, "bands": bands}

    kind = "D" if role == "Dimmer" else "V"
    return {"name": key, "role": role, "bands": [band(kind, 0, 255, "")]}


def band(kind, lo, hi, label):
    """One DMXpress band. OFL states 16-bit channel ranges at fine resolution
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
    # Most files express travel on the Pan/Tilt capability instead.
    for key, ch in (fixture.get("availableChannels") or {}).items():
        for c in caps_of(ch):
            if c.get("type") == "Pan" and pan is None:
                pan = abs(deg(c.get("angleEnd"), DEFAULT_PAN) - deg(c.get("angleStart"), 0.0))
            if c.get("type") == "Tilt" and tilt is None:
                tilt = abs(deg(c.get("angleEnd"), DEFAULT_TILT) - deg(c.get("angleStart"), 0.0))
    return (pan or DEFAULT_PAN, tilt or DEFAULT_TILT, beam)


def convert(root, out_path):
    manufacturers = json.load(open(os.path.join(root, "manufacturers.json")))
    entries, skipped_modes, skipped_fixtures = [], 0, 0

    for man_key in sorted(os.listdir(root)):
        man_dir = os.path.join(root, man_key)
        if not os.path.isdir(man_dir):
            continue
        man_name = (manufacturers.get(man_key) or {}).get("name", man_key)

        for fname in sorted(os.listdir(man_dir)):
            if not fname.endswith(".json"):
                continue
            fixture = json.load(open(os.path.join(man_dir, fname)))
            fix_key = fname[:-5]
            available = fixture.get("availableChannels") or {}

            # alias -> (parent channel key, parent role source)
            fine = {}
            for key, ch in available.items():
                for alias in ch.get("fineChannelAliases") or []:
                    roles = [cap_role(fixture, key, c) for c in caps_of(ch)]
                    named = [r for r in roles if r != "Other"]
                    fine[alias] = (key, named[0] if named else "Other")
            # Switching channels resolve to whichever channel they can become;
            # take the first, so at least the role is usually right.
            switching = {}
            for key, ch in available.items():
                for c in caps_of(ch):
                    for alias, target in (c.get("switchChannels") or {}).items():
                        switching.setdefault(alias, target)

            emitted_any = False
            for mode in fixture.get("modes") or []:
                slots = mode.get("channels") or []
                # Matrix/pixel modes splice in template blocks; expanding them
                # needs the matrix geometry, so they're left out for now.
                if any(isinstance(s, dict) for s in slots):
                    skipped_modes += 1
                    continue

                channels = []
                for i, slot in enumerate(slots):
                    if slot is None:
                        channels.append(
                            {"name": f"Unused {i + 1}", "role": "Other",
                             "bands": [band("V", 0, 255, "")]}
                        )
                        continue
                    if slot in available:
                        channels.append(channel_entry(fixture, slot, available[slot]))
                    elif slot in fine:
                        parent, parent_role = fine[slot]
                        channels.append(
                            channel_entry(fixture, slot, available[parent], fine_of=parent_role)
                        )
                    elif slot in switching and switching[slot] in available:
                        channels.append(
                            channel_entry(fixture, slot, available[switching[slot]])
                        )
                    else:
                        # Never drop a slot: the address span has to stay exact.
                        channels.append(
                            {"name": slot, "role": "Other", "bands": [band("V", 0, 255, "")]}
                        )

                if not channels:
                    continue
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

    entries.sort(key=lambda e: (e["manufacturer"].lower(), e["model"].lower(), len(e["channels"])))
    out = {
        "version": 1,
        "source": "Open Fixture Library (MIT) — github.com/OpenLightingProject/open-fixture-library",
        "fixtures": entries,
    }
    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    with open(out_path, "w") as fh:
        json.dump(out, fh, separators=(",", ":"))

    mans = len({e["manufacturer"] for e in entries})
    models = len({(e["manufacturer"], e["model"]) for e in entries})
    size = os.path.getsize(out_path) / 1e6
    print(f"{len(entries)} modes / {models} models / {mans} manufacturers -> {out_path} ({size:.1f} MB)")
    print(f"skipped {skipped_modes} matrix modes, {skipped_fixtures} fixtures with no usable mode")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    convert(sys.argv[1], sys.argv[2])
