#!/usr/bin/env python3
"""Adjudicate the locally authored strictness probe with the Zig oracle.

Usage: probe.py <inputs.txt> <oracle-exe> <repo> <commit> <zig-version>

The probe INPUTS are ours (test/strictness/inputs.txt, tracked). The VERDICTS
are not: every line is handed to the pinned Zig reference implementation, and
whatever it says is what the suite asserts. This repo never decides what ZON
means.
"""
import json, subprocess, sys

if len(sys.argv) != 6:
    sys.exit(__doc__)
inputs_path, oracle, repo, commit, zigver = sys.argv[1:]

ESC = {"n": "\n", "t": "\t", "\\": "\\"}


def unescape(s):
    out = []
    i = 0
    while i < len(s):
        if s[i] == "\\" and i + 1 < len(s) and s[i + 1] in ESC:
            out.append(ESC[s[i + 1]])
            i += 2
        else:
            out.append(s[i])
            i += 1
    return "".join(out)


srcs = []
for raw in open(inputs_path, encoding="utf8").read().split("\n"):
    if raw == "" or raw.startswith("#"):
        continue
    srcs.append(unescape(raw))

if not srcs:
    sys.exit("probe.py: no inputs")

blob = b"".join(
    (str(len(b)).encode() + b"\n" + b) for b in (s.encode("utf8") for s in srcs)
)
proc = subprocess.run([oracle], input=blob, stdout=subprocess.PIPE)
if proc.returncode != 0:
    sys.exit("probe.py: oracle exited %d" % proc.returncode)

lines = proc.stdout.decode("utf8").splitlines()
if len(lines) != len(srcs):
    sys.exit(
        "probe.py: oracle produced %d verdicts for %d inputs" % (len(lines), len(srcs))
    )


def _int(s):
    return float(s) if s == "-0" else int(s)


cases = []
for src, line in zip(srcs, lines):
    v = json.loads(line, parse_int=_int)
    c = {"source": src}
    if v["ok"]:
        c["valid"] = True
        c["value"] = v["value"]
    else:
        c["valid"] = False
        c["error"] = v["error"]
    cases.append(c)

json.dump(
    {
        "source": {
            "repo": repo,
            "commit": commit,
            "zig": zigver,
            "inputs": "test/strictness/inputs.txt (locally authored)",
            "note": "GENERATED, NEVER COMMITTED. Regenerate with scripts/fetch-zigzon.sh.",
        },
        "cases": cases,
    },
    sys.stdout,
    indent=1,
)
