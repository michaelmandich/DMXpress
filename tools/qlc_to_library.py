#!/usr/bin/env python3
"""Convert QLC+ fixture definitions into DMXpress library entries.

QLC+ (github.com/mcallegari/qlcplus, Apache-2.0) ships ~1700 `.qxf` files,
and unlike OFL it leans heavily on listing every channel-count mode a fixture
offers — the 6/7/8/13-channel variants of one par, the 15- and 30-channel
modes of one moving head. That is exactly the coverage OFL is thin on, so
DMXpress bundles both and `tools/build_library.py` merges them.

A `.qxf` declares its channels once and then lists, per `<Mode>`, which
channel sits at which DMX offset — the same shape as OFL, so the output here
is the same library entry: channels flattened into address order with an
explicit `role` each.

Usually run through `tools/build_library.py`. Standalone:

    curl -L -o qlc.tar.gz \\
        https://github.com/mcallegari/qlcplus/archive/refs/heads/master.tar.gz
    tar xzf qlc.tar.gz --strip-components=1 qlcplus-master/resources/fixtures
    python3 tools/qlc_to_library.py fixtures/ fixtures/qlc.json
"""

import os
import re
import sys
import xml.etree.ElementTree as ET

from ofl_to_library import MAX_CHANNELS, band, plain, write

NS = "{http://www.qlcplus.org/FixtureDefinition}"

DEFAULT_PAN = 540.0
DEFAULT_TILT = 270.0
DEFAULT_BEAM = 25.0

# QLC+ `<Channel Preset="...">` -> DMXpress Role. A preset is the definitive
# answer about what a channel does, so this is consulted before anything else.
PRESET_ROLE = {
    "IntensityDimmer": "Dimmer",
    "IntensityMasterDimmer": "Dimmer",
    # HSV/HSL controls: value and lightness are brightness, hue is a colour
    # slider, saturation on its own has no DMXpress equivalent.
    "IntensityValue": "Dimmer",
    "IntensityLightness": "Dimmer",
    "IntensityHue": "Color",
    "IntensityRed": "Red",
    "IntensityGreen": "Green",
    "IntensityBlue": "Blue",
    "IntensityWhite": "White",
    "IntensityAmber": "Amber",
    "IntensityUV": "Uv",
    "IntensityCyan": "Cyan",
    "IntensityMagenta": "Magenta",
    "IntensityYellow": "Yellow",
    # No role of their own; folding them into the neighbouring primary keeps
    # those emitters mixing into the visualiser instead of going dark.
    "IntensityLime": "Green",
    "IntensityIndigo": "Blue",
    "PositionPan": "Pan",
    "PositionTilt": "Tilt",
    "PositionPanFine": "PanFine",
    "PositionTiltFine": "TiltFine",
    # Scanner/laser X-Y positioning drives the same two axes.
    "PositionXAxis": "Pan",
    "PositionYAxis": "Tilt",
    "SpeedPanSlowFast": "Speed",
    "SpeedPanFastSlow": "Speed",
    "SpeedTiltSlowFast": "Speed",
    "SpeedTiltFastSlow": "Speed",
    "SpeedPanTiltSlowFast": "Speed",
    "SpeedPanTiltFastSlow": "Speed",
    "ColorMacro": "Color",
    "ColorWheel": "Color",
    "ColorRGBMixer": "Color",
    "ColorCTOMixer": "Color",
    "ColorCTCMixer": "Color",
    "ColorCTBMixer": "Color",
    "GoboWheel": "Gobo",
    "GoboIndex": "Gobo",
    "ShutterStrobeSlowFast": "Strobe",
    "ShutterStrobeFastSlow": "Strobe",
    "ShutterIrisMinToMax": "Iris",
    "ShutterIrisMaxToMin": "Iris",
    "BeamZoomSmallBig": "Zoom",
    "BeamZoomBigSmall": "Zoom",
    "BeamFocusNearFar": "Focus",
    "BeamFocusFarNear": "Focus",
    "PrismRotationSlowFast": "Prism",
    "PrismRotationFastSlow": "Prism",
    "NoFunction": "Other",
}

# Colour words that identify an emitter outright, longest first so "warm
# white" and "cold white" beat a bare "white" and "uv" doesn't match inside
# another word. Checked as whole words.
COLOR_WORDS = [
    ("warm white", "White"), ("cold white", "White"), ("cool white", "White"),
    ("magenta", "Magenta"), ("yellow", "Yellow"), ("indigo", "Blue"),
    ("amber", "Amber"), ("green", "Green"), ("white", "White"),
    ("cyan", "Cyan"), ("blue", "Blue"), ("lime", "Green"), ("red", "Red"),
    ("uv", "Uv"),
]

# Keywords safe to read off a channel name when the QLC+ group is one of the
# catch-alls (Effect/Maintenance/Nothing). Deliberately excludes pan, tilt
# and dimmer: an "Effect" channel called "Pan/Tilt macro" is a macro selector,
# and classifying it as Pan would make `stage::fixture::live_state` read the
# macro value as the head's position.
EFFECT_WORDS = [
    ("colour macro", "Color"), ("color macro", "Color"), ("colour", "Color"),
    ("color", "Color"), ("gobo", "Gobo"), ("prism", "Prism"),
    ("frost", "Frost"), ("iris", "Iris"), ("zoom", "Zoom"),
    ("focus", "Focus"), ("strobe", "Strobe"), ("speed", "Speed"),
]

# Roles whose stepped ranges DMXpress reads back — see `BANDED_ROLES` and
# `plain()` in ofl_to_library.py. Everything else is emitted bandless.
BANDED_ROLES = {"Color", "Gobo", "Dimmer"}

# QLC+ fixture types -> the OFL category vocabulary the rest of the library
# already uses. Anything unlisted passes through unchanged.
TYPE_CATEGORY = {
    "LED Bar (Beams)": "Pixel Bar",
    "LED Bar (Pixels)": "Pixel Bar",
    "Smoke": "Smoke",
    "Color Changer": "Color Changer",
}

def text(el):
    return (el.text or "").strip() if el is not None else ""


def whole_word(name, word):
    return re.search(rf"(?<![a-z]){re.escape(word)}(?![a-z])", name) is not None


def color_in(name):
    for word, role in COLOR_WORDS:
        if whole_word(name, word):
            return role
    return None


def group_role(group, byte, name):
    """Role from a channel's `<Group>` element, refined by its name."""
    if byte == 1:
        # Fine channels only have somewhere to put the low byte on pan/tilt.
        return {"Pan": "PanFine", "Tilt": "TiltFine"}.get(group, "Other")
    if group == "Intensity":
        return color_in(name) or "Dimmer"
    if group == "Colour":
        return color_in(name) or "Color"
    if group == "Shutter":
        # "Shutter & Strobing", "Strobing", "Strobe/Shutter"…
        return "Strobe" if "strob" in name else "Shutter"
    if group == "Beam":
        for word, role in EFFECT_WORDS:
            if whole_word(name, word):
                return role
        return "Other"
    if group in ("Gobo", "Prism", "Speed", "Pan", "Tilt"):
        return group
    # Effect / Maintenance / Nothing, plus anything unrecognised.
    for word, role in EFFECT_WORDS:
        if whole_word(name, word):
            return role
    return "Other"


def channel_role(ch):
    name = (ch.get("Name") or "").lower()
    preset = ch.get("Preset")
    if preset:
        if preset in PRESET_ROLE:
            return PRESET_ROLE[preset]
        # Every remaining preset is a `…Fine` variant of a mapped one. Only
        # pan and tilt fine bytes are usable, and those are mapped above.
        return "Other"
    group_el = ch.find(NS + "Group")
    byte = int(group_el.get("Byte") or 0) if group_el is not None else 0
    return group_role(text(group_el), byte, name)


def channel_entry(ch):
    """One DMXpress channel from a `<Channel>` element."""
    name = ch.get("Name") or "?"
    role = channel_role(ch)
    caps = ch.findall(NS + "Capability")

    if role in BANDED_ROLES and len(caps) > 1:
        bands = [
            band("S", cap.get("Min") or 0, cap.get("Max") or 255, text(cap))
            for cap in caps
        ]
        return {"name": name, "role": role, "bands": bands}

    return plain(name, role)


def physical(el):
    """(pan, tilt, beam) from a `<Physical>` element, or None for each part
    the element doesn't state."""
    if el is None:
        return (None, None, None)
    focus = el.find(NS + "Focus")
    lens = el.find(NS + "Lens")
    pan = tilt = beam = None
    if focus is not None:
        pan = float(focus.get("PanMax") or 0) or None
        tilt = float(focus.get("TiltMax") or 0) or None
    if lens is not None:
        degrees = [float(lens.get(k) or 0) for k in ("DegreesMin", "DegreesMax")]
        beam = max(degrees) or None
    return (pan, tilt, beam)


def geometry(root, mode):
    """Pan/tilt travel and beam angle, with mode-level physical overrides."""
    fix_pan, fix_tilt, fix_beam = physical(root.find(NS + "Physical"))
    pan, tilt, beam = physical(mode.find(NS + "Physical"))
    return (
        pan or fix_pan or DEFAULT_PAN,
        tilt or fix_tilt or DEFAULT_TILT,
        beam or fix_beam or DEFAULT_BEAM,
    )


def slug(value):
    """A stable, filename-ish id segment: 'Mega TRIPAR Profile' -> 'mega-tripar-profile'."""
    out = re.sub(r"[^a-z0-9]+", "-", str(value).lower()).strip("-")
    return out or "x"


def mode_channels(root, mode, channels):
    """The mode's channel list in address order, or None if it is unusable.

    `<Channel Number="n">` entries are placed by their stated offset rather
    than document order, and a gap or an unknown channel name still consumes
    its slot — the address span has to stay exact or every later fixture in
    the universe shifts.
    """
    placed = {}
    for ref in mode.findall(NS + "Channel"):
        try:
            index = int(ref.get("Number"))
        except (TypeError, ValueError):
            return None
        if index < 0 or index >= MAX_CHANNELS or index in placed:
            return None
        placed[index] = text(ref)
    if not placed:
        return None

    out = []
    for index in range(max(placed) + 1):
        name = placed.get(index)
        if name is None:
            out.append(plain(f"Unused {index + 1}", "Other"))
        elif name in channels:
            out.append(channel_entry(channels[name]))
        else:
            out.append(plain(name, "Other"))
    return out


def convert(root_dir):
    """Every patchable mode in a QLC+ `resources/fixtures/` tree."""
    entries, skipped_modes, duplicate_modes, skipped_files = [], 0, 0, 0
    seen_ids = set()

    for man_dir in sorted(os.listdir(root_dir)):
        path = os.path.join(root_dir, man_dir)
        if not os.path.isdir(path):
            continue
        for fname in sorted(os.listdir(path)):
            if not fname.endswith(".qxf"):
                continue
            try:
                root = ET.parse(os.path.join(path, fname)).getroot()
            except ET.ParseError as e:
                print(f"  ! {man_dir}/{fname}: {e}", file=sys.stderr)
                skipped_files += 1
                continue

            manufacturer = text(root.find(NS + "Manufacturer"))
            model = text(root.find(NS + "Model"))
            if not manufacturer or not model:
                skipped_files += 1
                continue
            kind = text(root.find(NS + "Type")) or "Other"
            channels = {
                ch.get("Name"): ch
                for ch in root.findall(NS + "Channel")
                if ch.get("Name")
            }

            emitted_any = False
            seen_modes = set()
            for mode in root.findall(NS + "Mode"):
                name = mode.get("Name") or ""
                built = mode_channels(root, mode, channels)
                if not built:
                    skipped_modes += 1
                    continue
                # A few .qxf files list the same mode twice. Nothing in the
                # patch browser could tell those two rows apart, so keep one.
                if (name.lower(), len(built)) in seen_modes:
                    duplicate_modes += 1
                    continue
                seen_modes.add((name.lower(), len(built)))
                pan, tilt, beam = geometry(root, mode)
                # Two modes of one fixture occasionally share a name at
                # different widths; the channel count keeps their ids apart.
                fid = f"qlcplus/{slug(manufacturer)}/{slug(model)}/{slug(name)}"
                if fid in seen_ids:
                    fid = f"{fid}-{len(built)}ch"
                while fid in seen_ids:
                    fid = f"{fid}-x"
                seen_ids.add(fid)
                entries.append({
                    "id": fid,
                    "manufacturer": manufacturer,
                    "model": model,
                    "mode": name,
                    "category": TYPE_CATEGORY.get(kind, kind),
                    "pan_range": round(pan, 1),
                    "tilt_range": round(tilt, 1),
                    "beam_width": round(beam, 1),
                    "channels": built,
                })
                emitted_any = True
            if not emitted_any:
                skipped_files += 1

    print(
        f"QLC+: {len(entries)} modes, skipped {skipped_modes} unusable modes, "
        f"{duplicate_modes} modes listed twice upstream and {skipped_files} "
        f"files with no usable mode",
        file=sys.stderr,
    )
    return entries


SOURCE = "QLC+ fixture definitions (Apache-2.0) — github.com/mcallegari/qlcplus"


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    write(convert(sys.argv[1]), sys.argv[2], SOURCE)
