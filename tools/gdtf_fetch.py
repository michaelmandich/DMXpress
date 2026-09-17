#!/usr/bin/env python3
"""Download GDTF fixture files from GDTF Share.

GDTF Share (gdtf-share.com) is where the manufacturers publish their own
fixture data. It is free, but the API needs an account, so this script asks
for your login rather than shipping one — nothing is stored and nothing is
written anywhere but the output directory.

    pip install pygdtf
    python3 tools/gdtf_fetch.py build/gdtf

It prompts for your GDTF Share email and password (the password is not
echoed). To run it unattended instead, set GDTF_USER and GDTF_PASSWORD in the
environment.

Downloading is resumable: files already in the output directory are skipped,
so a run that dies partway just needs running again. Only the newest revision
of each fixture is fetched.

    python3 tools/gdtf_fetch.py build/gdtf --manufacturer Robe --manufacturer Ayrton
    python3 tools/gdtf_fetch.py build/gdtf --list-manufacturers

Then convert what you got:

    python3 tools/gdtf_to_library.py build/gdtf fixtures/gdtf.json
"""

import argparse
import getpass
import http.cookiejar
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BASE = "https://gdtf-share.com/apis/public"

# GDTF Share is a free service run for the community; a full pull is thousands
# of requests, so leave a gap between them by default.
DEFAULT_DELAY = 0.4


def opener():
    """A URL opener that keeps the login session cookie."""
    jar = http.cookiejar.CookieJar()
    return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(client, slug, params=None, data=None):
    url = f"{BASE}/{slug}"
    if params:
        url += "?" + urllib.parse.urlencode(params)
    body = urllib.parse.urlencode(data).encode() if data else None
    request = urllib.request.Request(url, data=body)
    request.add_header("User-Agent", "DMXpress fixture library builder")
    with client.open(request, timeout=120) as response:
        return response.read()


def login(client, user, password):
    """True if GDTF Share accepted the login. A wrong password comes back as
    a plain 401, which urllib raises rather than returns."""
    try:
        raw = call(client, "login.php", data={"user": user, "password": password})
        answer = json.loads(raw.decode("utf-8", "replace"))
    except (urllib.error.URLError, OSError, ValueError):
        return False
    return bool(answer.get("result", False))


def catalogue(client):
    try:
        raw = call(client, "getList.php")
        return json.loads(raw.decode("utf-8", "replace")).get("list", []) or []
    except (urllib.error.URLError, OSError, ValueError) as e:
        sys.exit(f"Could not read the GDTF Share catalogue: {e}")


def newest_revisions(entries):
    """One entry per fixture — the highest revision id GDTF Share lists.

    The catalogue holds every revision ever uploaded, and downloading five
    versions of one Robe MegaPointe just to throw four away is rude to a free
    service and slow for you.
    """
    best = {}
    for entry in entries:
        try:
            rid = int(entry.get("rid"))
        except (TypeError, ValueError):
            continue
        key = (str(entry.get("manufacturer", "")), str(entry.get("fixture", "")))
        if key not in best or rid > int(best[key].get("rid")):
            best[key] = entry
    return list(best.values())


def safe(text):
    """A filename component that survives every filesystem."""
    return re.sub(r'[^A-Za-z0-9._-]+', "_", str(text)).strip("_") or "x"


def filename(entry):
    return (
        f"{safe(entry.get('manufacturer'))}@{safe(entry.get('fixture'))}"
        f"@{safe(entry.get('revision'))}.gdtf"
    )


def fetch(client, entries, out_dir, delay):
    os.makedirs(out_dir, exist_ok=True)
    saved = skipped = failed = 0
    total = len(entries)
    for i, entry in enumerate(entries, 1):
        path = os.path.join(out_dir, filename(entry))
        if os.path.exists(path) and os.path.getsize(path) > 0:
            skipped += 1
            continue
        try:
            blob = call(client, "downloadFile.php", params={"rid": entry.get("rid")})
        except (urllib.error.URLError, OSError) as e:
            print(f"  ! {entry.get('manufacturer')} {entry.get('fixture')}: {e}",
                  file=sys.stderr)
            failed += 1
            continue
        # A file too small to be a zip is an error page, not a fixture.
        if len(blob) < 200 or not blob.startswith(b"PK"):
            print(f"  ! {entry.get('manufacturer')} {entry.get('fixture')}: "
                  f"not a GDTF file ({len(blob)} bytes)", file=sys.stderr)
            failed += 1
            continue
        # Written under a temporary name first, so an interrupted run never
        # leaves a half file that the next run would skip as already done.
        tmp = path + ".part"
        with open(tmp, "wb") as fh:
            fh.write(blob)
        os.replace(tmp, path)
        saved += 1
        if saved % 25 == 0 or i == total:
            print(f"  {i}/{total} - {saved} saved, {skipped} already had, "
                  f"{failed} failed", file=sys.stderr)
        time.sleep(delay)
    return saved, skipped, failed


# GDTF Share lets anyone upload, and the "manufacturer" field is free text, so
# a slice of the catalogue is filed under a person's name, a placeholder or a
# test. These are not brands, and several have large enough counts that
# `--min-fixtures` can't tell them apart from a real small manufacturer.
# Applied by `--skip-junk`. Compared case-insensitively.
NOT_MANUFACTURERS = {
    "user test", "bakacowpoke", "brother brother and sons", "china",
    "fun_known", "fun_known_set", "set", "shinko tada", "alessandro",
    "ipablo", "samim reza", "reynolds", "markeeigenbau", "claclou",
    "erik nelson entertainment", "manzo", "theater_f", "obelie", "emc",
    "company na", "dgd gdtf", "hardwater", "dropit", "tableart",
    "ali express", "blenderdmx", "epic games", "vrsl",
}


def by_manufacturer(entries):
    counts = {}
    for entry in entries:
        maker = str(entry.get("manufacturer", "?"))
        counts[maker] = counts.get(maker, 0) + 1
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out_dir", help="directory to save .gdtf files into")
    parser.add_argument("--manufacturer", action="append", default=[],
                        help="only this maker (repeatable, case-insensitive)")
    parser.add_argument("--min-fixtures", type=int, default=0, metavar="N",
                        help="skip makers with fewer than N fixtures. Anyone may "
                             "upload to GDTF Share, so most of the ~1200 'makers' "
                             "are one-off personal files; 5 keeps the real ones")
    parser.add_argument("--skip-junk", action="store_true",
                        help="drop uploads filed under a person, a placeholder "
                             "or a test rather than a brand (see NOT_MANUFACTURERS)")
    parser.add_argument("--limit", type=int,
                        help="stop after this many files — use it to measure "
                             "download size and speed before committing to a full pull")
    parser.add_argument("--delay", type=float, default=DEFAULT_DELAY,
                        help=f"seconds between downloads (default {DEFAULT_DELAY})")
    parser.add_argument("--list-manufacturers", action="store_true",
                        help="print what's on offer and exit, downloading nothing")
    args = parser.parse_args()

    # Prompts and progress go to stderr so `--list-manufacturers > makers.txt`
    # still shows you the password prompt and still writes a clean file.
    def say(message):
        print(message, file=sys.stderr)

    user = os.environ.get("GDTF_USER")
    if not user:
        say("GDTF Share email: ")
        user = sys.stdin.readline().strip()
    password = os.environ.get("GDTF_PASSWORD") or getpass.getpass("GDTF Share password: ")
    if not user or not password:
        sys.exit("Need a GDTF Share login. Register free at https://gdtf-share.com/")

    client = opener()
    say("Logging in to GDTF Share...")
    if not login(client, user, password):
        sys.exit("GDTF Share rejected that login.")
    entries = catalogue(client)
    if not entries:
        sys.exit("GDTF Share returned an empty catalogue.")
    say(f"Catalogue: {len(entries)} revisions")

    wanted = newest_revisions(entries)
    if args.skip_junk:
        before = len(wanted)
        wanted = [e for e in wanted
                  if str(e.get("manufacturer", "")).strip().lower() not in NOT_MANUFACTURERS]
        say(f"--skip-junk: dropped {before - len(wanted)} fixtures filed under "
            f"a name that isn't a brand")
    counts = by_manufacturer(wanted)
    if args.min_fixtures > 1:
        small = {m for m, n in counts.items() if n < args.min_fixtures}
        wanted = [e for e in wanted if str(e.get("manufacturer", "")) not in small]
        say(f"--min-fixtures {args.min_fixtures}: dropped {len(small)} makers "
            f"with fewer than {args.min_fixtures} fixtures")
        counts = by_manufacturer(wanted)

    if args.list_manufacturers:
        for maker, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {n:5d}  {maker}")
        print(f"{len(counts)} manufacturers, {sum(counts.values())} fixtures")
        return

    if args.manufacturer:
        keep = {m.lower() for m in args.manufacturer}
        wanted = [e for e in wanted if str(e.get("manufacturer", "")).lower() in keep]
        missing = keep - {str(e.get("manufacturer", "")).lower() for e in wanted}
        if missing:
            say(f"  ! no such maker: {', '.join(sorted(missing))} "
                f"(names must match --list-manufacturers exactly)")
    wanted.sort(key=lambda e: (str(e.get("manufacturer", "")), str(e.get("fixture", ""))))
    if args.limit:
        wanted = wanted[: args.limit]
    if not wanted:
        sys.exit("Nothing selected to download.")
    say(f"Fetching {len(wanted)} fixtures into {args.out_dir}...")

    saved, skipped, failed = fetch(client, wanted, args.out_dir, args.delay)
    say(f"Done: {saved} downloaded, {skipped} already present, {failed} failed.")
    say(f"Next: python tools/gdtf_to_library.py {args.out_dir} fixtures/gdtf.json")


if __name__ == "__main__":
    main()
