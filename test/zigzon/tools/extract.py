#!/usr/bin/env python3
"""Extract ZON documents from a pinned ziglang/zig source tree.

Usage: extract.py <zig-source-root>   ->  JSON list on stdout

Sources, all upstream third-party (see scripts/fetch-zigzon.sh for the pin):
  * test/behavior/zon/*.zon             official ZON behaviour documents
  * test/cases/compile_errors/zon/*.zon official ZON must-fail documents
  * build.zig.zon, lib/init/build.zig.zon, src/codegen/**/*.zon  real manifests
  * lib/std/zon/parse.zig               ZON source literals inside `test` blocks

Emits [{name, origin, source}]. Validity and expected value are deliberately
NOT decided here -- tools/oracle.zig (the Zig reference implementation) decides
those. An extractor that also guessed the verdict would be marking its own
homework.
"""
import json, os, re, sys

if len(sys.argv) < 2:
    sys.exit("usage: extract.py <zig-source-root>")
ROOT = os.path.abspath(sys.argv[1])
if not os.path.isdir(ROOT):
    sys.exit("extract.py: not a directory: " + ROOT)

docs = []
seen = set()


def add(name, origin, source):
    # Deduplicate on source text: the same document appears in more than one
    # place upstream and would otherwise be counted twice, inflating the
    # denominator of the conformance figure.
    if source in seen:
        return
    seen.add(source)
    docs.append({"name": name, "origin": origin, "source": source})


# ---------------------------------------------------------------- .zon files
for path in sorted(
    os.path.join(dp, f)
    for dp, _, fs in os.walk(ROOT)
    for f in fs
    if f.endswith(".zon")
):
    origin = os.path.relpath(path, ROOT).replace(os.sep, "/")
    with open(path, "rb") as fh:
        raw = fh.read()
    try:
        src = raw.decode("utf8")
    except UnicodeDecodeError:
        src = raw.decode("latin1")
    add(origin, origin, src)


# ------------------------------------------------- Zig string literal decode
ZESC = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", "'": "'", '"': '"'}


def unzig(lit):
    """Decode the body of a Zig "..." string literal."""
    out = []
    i = 0
    while i < len(lit):
        c = lit[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        n = lit[i + 1]
        if n in ZESC:
            out.append(ZESC[n])
            i += 2
        elif n == "x":
            out.append(chr(int(lit[i + 2 : i + 4], 16)))
            i += 4
        elif n == "u":
            j = lit.index("}", i)
            out.append(chr(int(lit[i + 3 : j], 16)))
            i = j + 1
        else:
            raise ValueError("unknown escape " + repr(lit[i : i + 2]))
    return "".join(out)


def scan_string(text, i):
    """text[i] == '"'. Return (body, index-after-closing-quote)."""
    j = i + 1
    body = []
    while j < len(text):
        c = text[j]
        if c == "\\":
            body.append(text[j : j + 2])
            j += 2
            continue
        if c == '"':
            return "".join(body), j + 1
        if c == "\n":
            return None, j  # not a string literal after all
        body.append(c)
        j += 1
    return None, j


# ------------------------------------------------------------- parse.zig
PARSE = os.path.join(ROOT, "lib", "std", "zon", "parse.zig")
if not os.path.isfile(PARSE):
    sys.exit("extract.py: missing %s -- is the pinned source tree complete?" % PARSE)

text = open(PARSE, encoding="utf8").read()
lines = text.split("\n")

# Offsets of line starts, so an index into `text` maps back to a line number.
starts = []
off = 0
for ln in lines:
    starts.append(off)
    off += len(ln) + 1


def line_of(pos):
    lo, hi = 0, len(starts) - 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if starts[mid] <= pos:
            lo = mid
        else:
            hi = mid - 1
    return lo


CALL = re.compile(r"\bfromSlice(?:Alloc)?\s*\(")

for m in CALL.finditer(text):
    lineno = line_of(m.start())
    # Walk the argument list, tracking nesting, and collect top-level args.
    i = m.end()
    d = 1
    args = []
    cur_start = i
    while i < len(text) and d > 0:
        c = text[i]
        if c == '"':
            _, i = scan_string(text, i)
            continue
        if c in "([{":
            d += 1
            i += 1
            continue
        if c in ")]}":
            d -= 1
            if d == 0:
                args.append((cur_start, i))
                break
            i += 1
            continue
        if c == "," and d == 1:
            args.append((cur_start, i))
            cur_start = i + 1
            i += 1
            continue
        if c == "'":
            i += 2
            if i < len(text) and text[i - 1] == "\\":
                i += 1
            while i < len(text) and text[i] != "'":
                i += 1
            i += 1
            continue
        i += 1
    if len(args) < 3:
        continue
    a0, a1 = args[2]
    arg = text[a0:a1]
    stripped = arg.strip()
    src = None
    if stripped.startswith('"') and stripped.endswith('"') and len(stripped) >= 2:
        try:
            src = unzig(stripped[1:-1])
        except ValueError:
            src = None
    elif "\\\\" in stripped:
        # Zig multiline string literal: every line starts with `\\`.
        parts = []
        okay = True
        for ln in stripped.split("\n"):
            s = ln.strip()
            if s == "":
                continue
            if not s.startswith("\\\\"):
                okay = False
                break
            parts.append(s[2:])
        if okay and parts:
            src = "\n".join(parts)
    if src is None:
        continue
    where = "lib/std/zon/parse.zig:%d" % (lineno + 1)
    add(where, where, src)

if not docs:
    sys.exit("extract.py: extracted 0 documents -- the upstream layout changed")

json.dump(docs, sys.stdout)
sys.stderr.write("[extract] %d unique documents\n" % len(docs))
