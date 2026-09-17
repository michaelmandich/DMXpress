#!/usr/bin/env python3
"""Convert GDTF fixture files into DMXpress library entries.

GDTF (General Device Type Format) is the industry's own interchange format,
and unlike OFL and QLC+ the files are largely written by the manufacturers
themselves — which is why it is the only source that reliably carries the
current professional ranges: Robe's MegaPointe and BMFL, Ayrton's Khamsin and
Huracán, the High End SolaFrames, the Vari-Lite VL series.

Fetch the files first with `tools/gdtf_fetch.py` (GDTF Share needs a free
account), then:

    python3 tools/gdtf_to_library.py build/gdtf fixtures/gdtf.json

Normally run through `tools/build_library.py`, which merges this with OFL and
QLC+. Needs `pygdtf` (MIT): `pip install pygdtf`. GDTF's geometry-reference
mechanism repeats a block of channels once per pixel and interacts with the
DMX-break layout, which is far too subtle to reimplement — pygdtf resolves it
and hands back the flat, offset-resolved channel list this wants.
"""

import os
import re
import sys

try:
    import pygdtf
    from pygdtf import utils as gdtf_utils
except ImportError:  # pragma: no cover - a tooling-only dependency
    sys.exit("gdtf_to_library needs pygdtf: pip install pygdtf")

from ofl_to_library import MAX_CHANNELS, band, plain, write

DEFAULT_PAN = 540.0
DEFAULT_TILT = 270.0
DEFAULT_BEAM = 25.0

# GDTF attribute -> DMXpress Role, tried in order, first match wins. GDTF
# names attributes to a published standard and numbers repeated ones
# (`Gobo1`, `Gobo2`, `Shutter1Strobe`), so matching on shape generalises to
# the attributes this table has never seen — which matters, because the
# standard has several hundred and vendors add their own.
#
# `exact` entries must equal the attribute; `prefix` matches the start;
# `contains` matches anywhere.
ATTRIBUTE_RULES = [
    # Order matters enormously at the top. "PanRotate"/"TiltRotate" are
    # continuous-rotation *speed*, not position: classifying them as Pan/Tilt
    # would make `stage::fixture::live_state` read a spin rate as the head's
    # angle, because it takes the last Pan channel as the position.
    ("exact", "Pan", "Pan"),
    ("exact", "Tilt", "Tilt"),
    ("contains", "MSpeed", "Speed"),
    ("prefix", "PanTiltSpeed", "Speed"),
    ("contains", "Rotate", None),          # decided below, after the beam words
    ("exact", "Dimmer", "Dimmer"),
    ("exact", "HSB_Brightness", "Dimmer"),
    ("exact", "Intensity", "Dimmer"),
    # Beam optics before the generic colour catch-all, so "Gobo1Color" and
    # "Prism1Pos" land on the mechanism rather than on Color.
    ("prefix", "Gobo", "Gobo"),
    ("prefix", "AnimationWheel", "Gobo"),
    ("prefix", "Prism", "Prism"),
    ("prefix", "Frost", "Frost"),
    ("prefix", "Iris", "Iris"),
    ("prefix", "Zoom", "Zoom"),
    ("prefix", "Focus", "Focus"),
    # Strobe before shutter: "Shutter1Strobe*" is a rate, bare "Shutter1" is
    # the mechanical flag.
    ("contains", "Strobe", "Strobe"),
    ("prefix", "Shutter", "Shutter"),
    # Everything colour-ish: wheels, macros, correction filters, HSB/CIE.
    ("prefix", "CTO", "Color"),
    ("prefix", "CTC", "Color"),
    ("prefix", "CTB", "Color"),
    ("prefix", "CTF", "Color"),
    ("prefix", "ColorTemperature", "Color"),
    ("prefix", "HSB_", "Color"),
    ("prefix", "CIE_", "Color"),
    ("prefix", "Color", "Color"),
]

# Attributes matching "Rotate" resolve against these first — a rotating gobo
# is still a gobo, but a rotating *head* is a speed, not a position.
ROTATE_ROLES = [
    ("Gobo", "Gobo"),
    ("Prism", "Prism"),
    ("AnimationWheel", "Gobo"),
    ("Color", "Color"),
]

# One emitter per `ColorAdd_*` / `ColorSub_*` / `ColorRGB_*` suffix. GDTF's
# standard spells these as letter codes, but `ColorRGB_Green` and friends turn
# up in plenty of real files, so both spellings map. The blends fold into the
# nearest primary the visualiser can actually render, the way the OFL
# converter already folds Lime into Green.
EMITTERS = {
    "R": "Red", "RED": "Red",
    "G": "Green", "GREEN": "Green",
    "B": "Blue", "BLUE": "Blue",
    "W": "White", "WHITE": "White",
    "WW": "White", "WARMWHITE": "White",
    "CW": "White", "COOLWHITE": "White", "COLDWHITE": "White",
    "A": "Amber", "AMBER": "Amber",
    "UV": "Uv",
    "C": "Cyan", "CYAN": "Cyan",
    "M": "Magenta", "MAGENTA": "Magenta",
    "Y": "Yellow", "YELLOW": "Yellow",
    "RY": "Amber",                      # red-yellow reads as amber
    "GY": "Green", "LIME": "Green",
    "BG": "Cyan",
    "RB": "Magenta", "INDIGO": "Blue", "PURPLE": "Magenta",
}

# `ColorAdd_R`, `ColorSub_C`, `ColorRGB_Green`… — anything else after the
# underscore (`ColorAdd_RGB`, a gamut name) is a colour control, not one
# emitter, and falls through to the ordinary rules.
EMITTER_ATTRIBUTE = re.compile(r"^Color(?:Add|Sub|RGB)_([A-Za-z]+)$")


def emitter_role(name):
    """Role for a single-emitter colour attribute, else None."""
    match = EMITTER_ATTRIBUTE.match(name)
    return EMITTERS.get(match.group(1).upper()) if match else None

# Roles whose stepped ranges DMXpress reads back — see `BANDED_ROLES` and
# `plain()` in ofl_to_library.py. Everything else is emitted bandless.
BANDED_ROLES = {"Color", "Gobo", "Dimmer"}

# Geometry names that say nothing about which emitter a channel belongs to,
# so the channel is better named after its attribute alone.
GENERIC_GEOMETRIES = {
    "base", "body", "head", "yoke", "beam", "main", "pan", "tilt",
    "geometry", "fixture", "lamp", "module", "default",
}


def attribute_role(attribute):
    """The DMXpress role for a GDTF attribute name."""
    name = str(attribute or "")
    if not name or name == "NoFeature":
        return "Other"
    emitter = emitter_role(name)
    if emitter:
        return emitter
    for kind, needle, role in ATTRIBUTE_RULES:
        hit = (
            name == needle if kind == "exact"
            else name.startswith(needle) if kind == "prefix"
            else needle in name
        )
        if not hit:
            continue
        if role is not None:
            return role
        # The "Rotate" rule: what is rotating decides.
        for prefix, rotate_role in ROTATE_ROLES:
            if name.startswith(prefix):
                return rotate_role
        return "Speed"
    return "Other"


def channel_name(channel):
    """A readable per-slot name.

    pygdtf resolves geometry references, so a pixel bar's channels arrive with
    the instance geometry attached — "Pixel 3". That is worth keeping; a plain
    "Head" or "Base" is not.
    """
    attribute = str(channel.attribute or "Unknown")
    if attribute == "NoFeature":
        # GDTF's name for a reserved slot. It still costs an address.
        return "No function"
    geometry = str(channel.geometry or "")
    if geometry and geometry.lower() not in GENERIC_GEOMETRIES:
        return f"{geometry} {attribute}".strip()
    return attribute


def dmx8(value):
    """A GDTF DMX value as a single slot's 0-255, whatever byte count it was
    stated at (GDTF writes 16-bit ranges at 16-bit resolution)."""
    raw = getattr(value, "value", value)
    try:
        raw = int(raw)
    except (TypeError, ValueError):
        return None
    while raw > 255:
        raw >>= 8
    return max(0, min(255, raw))


def channel_bands(channel):
    """Labelled sub-ranges for a wheel, macro or split dimmer channel."""
    bands = []
    for logical in channel.logical_channels or []:
        for function in logical.channel_functions or []:
            sets = function.channel_sets or []
            if sets:
                for entry in sets:
                    lo = dmx8(entry.dmx_from)
                    hi = dmx8(getattr(entry, "dmx_to", None))
                    if lo is None:
                        continue
                    label = str(entry.name or function.name or "")
                    bands.append(band("S", lo, 255 if hi is None else hi, label))
            else:
                lo = dmx8(function.dmx_from)
                hi = dmx8(getattr(function, "dmx_to", None))
                if lo is not None:
                    bands.append(
                        band("S", lo, 255 if hi is None else hi, str(function.name or ""))
                    )
    return bands


def physical_span(channel):
    """How far a Pan/Tilt channel actually travels, in degrees."""
    for logical in channel.logical_channels or []:
        for function in logical.channel_functions or []:
            lo = getattr(function.physical_from, "value", None)
            hi = getattr(function.physical_to, "value", None)
            if lo is not None and hi is not None and hi != lo:
                return abs(float(hi) - float(lo))
    return None


def beam_angle(fixture, mode_name):
    """Widest beam angle among the mode's beam geometries."""
    try:
        beams = gdtf_utils.get_beam_geometries_for_mode(fixture, mode_name)
    except Exception:
        return None
    angles = []
    for beam in beams or []:
        for attr in ("beam_angle", "field_angle"):
            value = getattr(beam, attr, None)
            if value:
                try:
                    angles.append(float(value))
                except (TypeError, ValueError):
                    pass
    return max(angles) if angles else None


def build_mode(fixture, mode, channels):
    """One mode's channel list in address order, or None if unusable."""
    slots, pan, tilt = {}, None, None
    for channel in channels:
        offsets = [o for o in (channel.offset or []) if isinstance(o, int) and o > 0]
        if not offsets:
            continue  # a virtual channel: real control, but no DMX slot
        role = attribute_role(channel.attribute)
        name = channel_name(channel)
        if role == "Pan" and pan is None:
            pan = physical_span(channel)
        elif role == "Tilt" and tilt is None:
            tilt = physical_span(channel)

        bands = channel_bands(channel) if role in BANDED_ROLES else []
        coarse = ({"name": name, "role": role, "bands": bands} if len(bands) > 1
                  else plain(name, role))
        if offsets[0] in slots:
            return None, None, None  # two channels claiming one slot
        slots[offsets[0]] = coarse
        # Only pan and tilt have anywhere to put the low byte; every other
        # fine channel stays unclassified so it can't fight its coarse half.
        fine_role = {"Pan": "PanFine", "Tilt": "TiltFine"}.get(role, "Other")
        for offset in offsets[1:]:
            if offset in slots:
                return None, None, None
            slots[offset] = plain(f"{name} fine", fine_role)
            fine_role = "Other"  # a third byte is ultra, not fine

    if not slots or max(slots) > MAX_CHANNELS:
        return None, None, None
    built = [
        slots.get(i) or plain(f"Unused {i}", "Other")
        for i in range(1, max(slots) + 1)
    ]
    return built, pan, tilt


def slug(value):
    out = re.sub(r"[^a-z0-9]+", "-", str(value).lower()).strip("-")
    return out or "x"


def convert(root_dir):
    """Every patchable mode in a directory of `.gdtf` files."""
    entries, skipped_modes, skipped_files, multi_break = [], 0, 0, 0
    unmapped = {}
    seen_ids = set()

    paths = []
    for dirpath, _, filenames in os.walk(root_dir):
        paths += [os.path.join(dirpath, f) for f in filenames if f.lower().endswith(".gdtf")]

    for path in sorted(paths):
        try:
            fixture = pygdtf.FixtureType(path)
        except Exception as e:
            print(f"  ! {os.path.basename(path)}: {e}", file=sys.stderr)
            skipped_files += 1
            continue
        manufacturer = (fixture.manufacturer or "").strip()
        model = (fixture.long_name or fixture.name or "").strip()
        if not manufacturer or not model:
            skipped_files += 1
            continue

        emitted_any = False
        seen_modes = set()
        for mode in fixture.dmx_modes or []:
            mode_name = (mode.name or "Default").strip()
            try:
                breaks = gdtf_utils.get_dmx_channels(fixture, mode.name)
            except Exception:
                skipped_modes += 1
                continue
            # A multi-break mode wants two or more separate DMX addresses.
            # DMXpress patches one contiguous span, so it can't hold one.
            if len(breaks) != 1:
                multi_break += 1
                continue

            built, pan, tilt = build_mode(fixture, mode, breaks[0])
            if not built:
                skipped_modes += 1
                continue
            if (mode_name.lower(), len(built)) in seen_modes:
                skipped_modes += 1
                continue
            seen_modes.add((mode_name.lower(), len(built)))

            for channel in breaks[0]:
                attribute = str(channel.attribute or "")
                if attribute and attribute_role(attribute) == "Other":
                    unmapped[attribute] = unmapped.get(attribute, 0) + 1

            fid = f"gdtf/{slug(manufacturer)}/{slug(model)}/{slug(mode_name)}"
            if fid in seen_ids:
                fid = f"{fid}-{len(built)}ch"
            while fid in seen_ids:
                fid = f"{fid}-x"
            seen_ids.add(fid)
            entries.append({
                "id": fid,
                "manufacturer": manufacturer,
                "model": model,
                "mode": mode_name,
                "category": "Other",
                "pan_range": round(pan or DEFAULT_PAN, 1),
                "tilt_range": round(tilt or DEFAULT_TILT, 1),
                "beam_width": round(beam_angle(fixture, mode.name) or DEFAULT_BEAM, 1),
                "channels": built,
            })
            emitted_any = True
        if not emitted_any:
            skipped_files += 1

    print(
        f"GDTF: {len(entries)} modes from {len(paths)} files, skipped "
        f"{skipped_modes} unusable modes, {multi_break} multi-break modes and "
        f"{skipped_files} files with no usable mode",
        file=sys.stderr,
    )
    if unmapped:
        top = sorted(unmapped.items(), key=lambda kv: -kv[1])[:12]
        print(f"  {len(unmapped)} attributes fell through to Other, most common: "
              + ", ".join(f"{a}({n})" for a, n in top), file=sys.stderr)
    return entries


SOURCE = "GDTF Share — gdtf-share.com, per-file licences from each manufacturer"

# What `attribute_role` has to say about the attributes that matter, checked
# by `--check`. Mostly guarding the order-sensitive cases: a rotating gobo is
# a gobo but a rotating head is a speed, a strobe rate beats a bare shutter,
# and `DimmerCurve` is a curve selector rather than a second dimmer.
SELF_TEST = [
    ("Pan", "Pan"), ("Tilt", "Tilt"),
    ("PanRotate", "Speed"), ("TiltRotate", "Speed"),
    ("PositionMSpeed", "Speed"), ("PanTiltSpeed", "Speed"),
    ("Dimmer", "Dimmer"), ("DimmerCurve", "Other"), ("HSB_Brightness", "Dimmer"),
    ("ColorAdd_R", "Red"), ("ColorAdd_G", "Green"), ("ColorAdd_B", "Blue"),
    ("ColorAdd_W", "White"), ("ColorAdd_A", "Amber"), ("ColorAdd_UV", "Uv"),
    ("ColorAdd_GY", "Green"), ("ColorAdd_RY", "Amber"),
    ("ColorRGB_Green", "Green"), ("ColorSub_C", "Cyan"), ("ColorSub_M", "Magenta"),
    ("ColorAdd_RGB", "Color"), ("Color1", "Color"), ("ColorMacro1", "Color"),
    ("CTO", "Color"), ("ColorTemperature", "Color"),
    ("Gobo1", "Gobo"), ("Gobo2Pos", "Gobo"), ("Gobo1PosRotate", "Gobo"),
    ("Gobo1WheelSpin", "Gobo"), ("AnimationWheel1", "Gobo"),
    ("Shutter1", "Shutter"), ("Shutter1Strobe", "Strobe"),
    ("Shutter1StrobeRate", "Strobe"),
    ("Prism1", "Prism"), ("Prism1PosRotate", "Prism"),
    ("Frost1", "Frost"), ("Iris", "Iris"), ("Zoom", "Zoom"), ("Focus1", "Focus"),
    ("NoFeature", "Other"), ("Blade1A", "Other"), ("Control1", "Other"),
    ("", "Other"),
]


def self_check():
    bad = [(a, want, attribute_role(a)) for a, want in SELF_TEST
           if attribute_role(a) != want]
    for attribute, want, got in bad:
        print(f"  {attribute!r}: expected {want}, got {got}", file=sys.stderr)
    print(f"attribute_role: {len(SELF_TEST) - len(bad)}/{len(SELF_TEST)} as expected")
    return 1 if bad else 0


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--check":
        sys.exit(self_check())
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    write(convert(sys.argv[1]), sys.argv[2], SOURCE)
