"""Audio device tuner for SigmaOS-audio-tuner (inspired by omarchy-audio-tuner).

Principles: preset maths is a pure function of frequency (deterministic,
testable EQ band plans), device discovery is a pure parser over captured tool
output (self_test never talks to PipeWire/PulseAudio), profiles are validated
JSON documents, fail-fast range checks everywhere.
"""
import argparse, json, math, os, re, sys

ENGINE = {
    "name": "sigma-audio",
    "summary": "Audio tuner: devices, profiles, EQ band plans, balance",
    "language": "python",
    "verbs": ["devices", "profile", "eq-plan", "balance"],
    "principles": ["Separation of Concerns", "Abstraction",
                   "Defensive Programming", "Fail-Fast", "KISS",
                   "Deterministic Testing"],
}

cli_doc = """\
$ sigma-audio devices --input sinks.txt
1  alsa_output.pci-0000_00_1f.3.analog-stereo  RUNNING
$ sigma-audio eq-plan bass-boost
31.25 Hz  +5.5 dB  q=1.20
$ sigma-audio eq-plan vocal --format eqapo
$ sigma-audio balance -50
left=1.00 (-0.0 dB)  right=0.50 (-6.0 dB)
$ sigma-audio profile --name desk --sink alsa_output.pci --save desk.json
"""

# 10-band ISO octave centres with per-band Q
BAND_FREQS = (31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0,
              8000.0, 16000.0)
BAND_Q = (1.2, 1.2, 1.1, 1.0, 1.0, 1.0, 0.9, 0.9, 0.8, 0.8)


class AudioError(ValueError):
    pass


# --------------------------------------------------------------------------- #
#  Device discovery: pure parser over captured `pactl` output
# --------------------------------------------------------------------------- #
def parse_devices(text):
    """Parse `pactl list short sinks` style TSV: id, name, driver, spec, state."""
    if not isinstance(text, str):
        raise AudioError("expected text input")
    out = []
    for lineno, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) < 5:
            raise AudioError(f"line {lineno}: expected 5 tab-separated "
                             f"fields, got {len(fields)}")
        dev_id, name, driver, spec, state = fields[:5]
        if not dev_id.isdigit():
            raise AudioError(f"line {lineno}: bad device id {dev_id!r}")
        if not name.strip() or not name.startswith(("alsa_", "bluez",
                                                    "api_", "spa_")):
            raise AudioError(f"line {lineno}: unexpected device name {name!r}")
        out.append({"id": int(dev_id), "name": name, "driver": driver,
                    "spec": spec, "state": state})
    return out


# --------------------------------------------------------------------------- #
#  Profiles (validated JSON documents)
# --------------------------------------------------------------------------- #
_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")


def profile(name, sink, eq_preset="flat", balance=0):
    if not isinstance(name, str) or not _NAME_RE.match(name):
        raise AudioError(f"bad profile name: {name!r}")
    if not isinstance(sink, str) or not sink.strip():
        raise AudioError(f"bad sink: {sink!r}")
    if "\n" in sink or "\x00" in sink:
        raise AudioError("sink must be a single line")
    if eq_preset not in PRESETS:
        raise AudioError(f"unknown EQ preset {eq_preset!r} "
                         f"(have: {', '.join(sorted(PRESETS))})")
    balance = _check_balance(balance)
    return {"name": name, "sink": sink, "eq_preset": eq_preset,
            "balance": balance}


def save_profile(path, prof):
    tmp = f"{path}.sigma-tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(prof, fh, indent=2, sort_keys=True)
        fh.write("\n")
    os.replace(tmp, path)


def load_profile(path):
    with open(path, "r", encoding="utf-8") as fh:
        try:
            data = json.load(fh)
        except json.JSONDecodeError as e:
            raise AudioError(f"profile is not valid JSON: {e}") from e
    if not isinstance(data, dict):
        raise AudioError("profile must be a JSON object")
    unknown = set(data) - {"name", "sink", "eq_preset", "balance"}
    if unknown:
        raise AudioError(f"unknown profile field(s): {sorted(unknown)}")
    for field in ("name", "sink"):
        if field not in data:
            raise AudioError(f"profile missing {field!r}")
    return profile(data["name"], data["sink"],
                   data.get("eq_preset", "flat"), data.get("balance", 0))


# --------------------------------------------------------------------------- #
#  EQ band plans (pure maths)
# --------------------------------------------------------------------------- #
def _g_flat(f):
    return 0.0


def _g_bass(f):
    return round(6.0 * max(0.0, 1.0 - f / 400.0), 1)


def _g_treble(f):
    return round(5.0 * min(1.0, max(0.0, math.log2(f / 2000.0))), 1)


def _g_vocal(f):
    return round(4.0 * 2.0 ** (-(math.log2(f / 1500.0) ** 2)), 1)


def _g_night(f):
    if f <= 125.0:
        return -2.0
    if f >= 2000.0:
        return round(-min(6.0, 3.0 * math.log2(f / 1000.0)), 1)
    return 0.0


PRESETS = {
    "flat": _g_flat,
    "bass-boost": _g_bass,
    "treble-lift": _g_treble,
    "vocal": _g_vocal,
    "night": _g_night,
}


def eq_plan(preset, freqs=None):
    """Deterministic list of {freq, gain_db, q} band dicts for a preset."""
    if preset not in PRESETS:
        raise AudioError(f"unknown EQ preset {preset!r} "
                         f"(have: {', '.join(sorted(PRESETS))})")
    gain = PRESETS[preset]
    if freqs is None:
        freqs = list(BAND_FREQS)
    if not isinstance(freqs, (list, tuple)) or not 4 <= len(freqs) <= 31:
        raise AudioError("freqs must be a list of 4..31 band centres")
    prev = 0.0
    out = []
    for i, f in enumerate(freqs):
        if not isinstance(f, (int, float)) or isinstance(f, bool) or f <= 0:
            raise AudioError(f"bad band frequency: {f!r}")
        if f <= prev:
            raise AudioError("band frequencies must be strictly ascending")
        prev = float(f)
        q = BAND_Q[i] if i < len(BAND_Q) else 1.0
        out.append({"freq": float(f), "gain_db": gain(float(f)), "q": q})
    return out


def eq_apo(plan, preset="flat"):
    """Render an EqualizerAPO command block from a band plan."""
    lines = [f"# sigma-audio-tuner preset={preset}"]
    for band in plan:
        lines.append(f"filter ON PEQU 0 {band['freq']:g} "
                     f"{band['gain_db']:+.1f} {band['q']:.2f}")
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------- #
#  Channel balance (pure)
# --------------------------------------------------------------------------- #
def _check_balance(pct):
    if isinstance(pct, str):
        try:
            pct = float(pct.strip())
        except ValueError as e:
            raise AudioError(f"balance wants a number, got {pct!r}") from e
    if isinstance(pct, bool) or not isinstance(pct, (int, float)):
        raise AudioError(f"balance wants a number, got {pct!r}")
    if not -100 <= pct <= 100:
        raise AudioError(f"balance must be -100..100, got {pct}")
    return float(pct)


def balance(pct):
    """-100 = full left, 0 = centred, +100 = full right. Returns linear + dB."""
    pct = _check_balance(pct)
    if pct <= 0:
        left, right = 1.0, 1.0 + pct / 100.0
    else:
        left, right = 1.0 - pct / 100.0, 1.0
    left_db = round(20.0 * math.log10(left), 2) if left > 0 else None
    right_db = round(20.0 * math.log10(right), 2) if right > 0 else None
    return {"balance": pct, "left": round(left, 4), "right": round(right, 4),
            "left_db": left_db, "right_db": right_db}


# --------------------------------------------------------------------------- #
#  CLI
# --------------------------------------------------------------------------- #
def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog=ENGINE["name"], description=ENGINE["summary"])
    sub = p.add_subparsers(dest="cmd", required=True)
    dv = sub.add_parser("devices", help="parse captured pactl short output")
    dv.add_argument("--input", default="-", help="file with pactl output ('-' = stdin)")
    pf = sub.add_parser("profile", help="build/load a profile document")
    pf.add_argument("--name", default=None)
    pf.add_argument("--sink", default=None)
    pf.add_argument("--eq-preset", default="flat")
    pf.add_argument("--balance", default="0")
    pf.add_argument("--save", default=None, metavar="FILE")
    pf.add_argument("--load", default=None, metavar="FILE")
    eq = sub.add_parser("eq-plan", help="compute the EQ band plan for a preset")
    eq.add_argument("preset", choices=sorted(PRESETS))
    eq.add_argument("--freqs", default=None,
                    help="comma-separated custom band centres")
    eq.add_argument("--format", default="table", choices=("table", "eqapo", "json"))
    bl = sub.add_parser("balance", help="compute channel balance")
    bl.add_argument("percent", help="-100 (full left) .. +100 (full right)")
    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.cmd == "devices":
            text = sys.stdin.read() if args.input == "-" else open(
                args.input, "r", encoding="utf-8").read()
            for d in parse_devices(text):
                print(f"{d['id']:<4} {d['name']:<48} {d['state']}")
        elif args.cmd == "profile":
            if args.load:
                prof = load_profile(args.load)
            else:
                if not args.name or not args.sink:
                    raise AudioError("--name and --sink are required "
                                     "(or use --load)")
                prof = profile(args.name, args.sink, args.eq_preset,
                               args.balance)
            if args.save:
                save_profile(args.save, prof)
            print(json.dumps(prof, indent=2, sort_keys=True))
        elif args.cmd == "eq-plan":
            freqs = None
            if args.freqs:
                freqs = [float(x) for x in args.freqs.split(",")]
            plan = eq_plan(args.preset, freqs)
            if args.format == "json":
                print(json.dumps(plan, indent=2))
            elif args.format == "eqapo":
                print(eq_apo(plan, args.preset), end="")
            else:
                for band in plan:
                    print(f"{band['freq']:>7.2f} Hz  "
                          f"{band['gain_db']:+.1f} dB  q={band['q']:.2f}")
        elif args.cmd == "balance":
            b = balance(args.percent)
            l = "-inf" if b["left_db"] is None else f"{b['left_db']:.1f}"
            r = "-inf" if b["right_db"] is None else f"{b['right_db']:.1f}"
            print(f"left={b['left']:.2f} ({l} dB)  "
                  f"right={b['right']:.2f} ({r} dB)")
        return 0
    except (AudioError, OSError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


def self_test() -> None:
    import tempfile

    # --- device parsing (pure, realistic pactl output) ----------------------
    SINKS = ("5\talsa_output.pci-0000_00_1f.3.analog-stereo\t"
             "module-alsa-card.c\ts32le 2ch 48000Hz\tRUNNING\n"
             "7\talsa_output.usb-JBL_107_0001.stereo\t"
             "module-alsa-card.c\ts16le 2ch 44100Hz\tIDLE\n"
             "\n"
             "9\tbluez_sink.40_ED_98_A1_B2_C3.1\t"
             "module-bluez5-device.c\ts16le 2ch 44100Hz\tSUSPENDED\n")
    devs = parse_devices(SINKS)
    assert [d["id"] for d in devs] == [5, 7, 9]
    assert devs[0]["name"] == "alsa_output.pci-0000_00_1f.3.analog-stereo"
    assert devs[0]["state"] == "RUNNING" and devs[1]["state"] == "IDLE"
    assert devs[2]["name"].startswith("bluez_sink.")
    assert devs[1]["spec"] == "s16le 2ch 44100Hz"
    assert parse_devices("") == []
    for bad in ("5\tonly\tthree\n",                 # too few fields
                "xx\talsa_output.x\tm\tc\tRUNNING\n",  # bad id
                "5\tweird-name\tm\tc\tRUNNING\n"):      # bad name prefix
        try:
            parse_devices(bad)
        except AudioError:
            pass
        else:
            raise AssertionError(f"devices should have been rejected: {bad!r}")

    # --- profiles ------------------------------------------------------------
    prof = profile("desk", "alsa_output.pci-0000_00_1f.3.analog-stereo",
                   "bass-boost", -20)
    assert prof["eq_preset"] == "bass-boost" and prof["balance"] == -20.0
    for bad_call in ((profile, ("bad name!", "sink")),
                     (profile, ("p", "")),
                     (profile, ("p", "s", "mega-boost")),
                     (profile, ("p", "s", "flat", 200)),
                     (profile, ("p", "s", "flat", -200)),
                     (profile, ("p", "s", "flat", "loud"))):
        try:
            bad_call[0](*bad_call[1])
        except AudioError:
            pass
        else:
            raise AssertionError(f"should have rejected: {bad_call}")
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "desk.json")
        save_profile(path, prof)
        assert load_profile(path) == prof
        with open(path, "r", encoding="utf-8") as fh:
            assert json.load(fh)["sink"].startswith("alsa_output.")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write("not json")
        try:
            load_profile(path)
        except AudioError:
            pass
        else:
            raise AssertionError("bad profile JSON not rejected")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write('{"name": "x", "sink": "y", "gain": 3}')
        try:
            load_profile(path)
        except AudioError:
            pass
        else:
            raise AssertionError("unknown profile field not rejected")

    # --- EQ plans ------------------------------------------------------------
    flat = eq_plan("flat")
    assert len(flat) == 10
    assert [b["freq"] for b in flat] == list(BAND_FREQS)
    assert all(b["gain_db"] == 0.0 for b in flat)
    assert all(b["q"] > 0 for b in flat)
    bass = [b["gain_db"] for b in eq_plan("bass-boost")]
    assert bass[0] == 5.5, bass            # 31.25 Hz: ~+5.5 dB
    assert bass[4] == 0.0 and bass[9] == 0.0   # mids/highs untouched
    assert all(bass[i] >= bass[i + 1] for i in range(len(bass) - 1))  # monotone
    treble = [b["gain_db"] for b in eq_plan("treble-lift")]
    assert treble[0] == 0.0 and treble[5] == 0.0
    assert treble[7] == 5.0 and treble[9] == 5.0   # 4k+ lifted, clamped
    vocal = [b["gain_db"] for b in eq_plan("vocal")]
    assert vocal[0] == 0.0 and vocal[9] == 0.0      # ends flat
    assert vocal[5] > 0 and vocal[6] > 0            # 1k-2k region up
    assert max(vocal) <= 4.0
    night = [b["gain_db"] for b in eq_plan("night")]
    assert night[0] == -2.0 and night[3] == 0.0
    assert night[7] == -6.0 and night[9] == -6.0    # highs rolled off & clamped
    assert eq_plan("flat") == flat                  # deterministic

    # custom band list
    custom = eq_plan("bass-boost", freqs=[100, 200, 400, 800])
    assert [b["freq"] for b in custom] == [100.0, 200.0, 400.0, 800.0]
    assert custom[3]["gain_db"] == 0.0              # 800 > 400 -> 0 dB
    for bad_call in ((eq_plan, ("mega-boost",)),
                     (eq_plan, ("flat", [400, 200])),
                     (eq_plan, ("flat", [0, 100, 200, 300])),
                     (eq_plan, ("flat", [100, 100, 200, 300])),
                     (eq_plan, ("flat", [100])),
                     (eq_plan, ("flat", "100,200")),
                     (eq_plan, ("flat", [100, True, 200, 300]))):
        try:
            bad_call[0](*bad_call[1])
        except AudioError:
            pass
        else:
            raise AssertionError(f"should have rejected: {bad_call}")

    # EQ-APO rendering
    text = eq_apo(flat, "flat")
    assert text.startswith("# sigma-audio-tuner preset=flat")
    assert "filter ON PEQU 0 31.25 +0.0 1.20" in text, text
    assert "filter ON PEQU 0 16000 +0.0 0.80" in text
    assert text == eq_apo(flat, "flat")            # deterministic
    bass_text = eq_apo(eq_plan("bass-boost"), "bass-boost")
    assert "+5.5" in bass_text

    # --- balance -------------------------------------------------------------
    b = balance(0)
    assert b["left"] == 1.0 and b["right"] == 1.0
    assert b["left_db"] == 0.0 and b["right_db"] == 0.0
    b = balance(-100)
    assert b["left"] == 1.0 and b["right"] == 0.0 and b["right_db"] is None
    b = balance(100)
    assert b["left"] == 0.0 and b["right"] == 1.0 and b["left_db"] is None
    b = balance(50)
    assert b["left"] == 0.5 and b["right"] == 1.0
    assert abs(b["left_db"] - (-6.02)) < 0.01
    b = balance(-50)
    assert b["left"] == 1.0 and b["right"] == 0.5
    b = balance("50")
    assert b["left"] == 0.5
    for bad in (150, -150, "loud", None, True):
        try:
            balance(bad)
        except AudioError:
            pass
        else:
            raise AssertionError(f"balance should have been rejected: {bad!r}")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
