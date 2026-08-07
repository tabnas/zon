#!/usr/bin/env python3
"""Run every extracted document through the Zig oracle and emit cases.json.

Usage: adjudicate.py <docs.json> <oracle-exe> <repo> <commit> <zig-version>

The verdict (valid / invalid) and, for valid documents, the expected value both
come from the oracle -- i.e. from the pinned Zig reference implementation. This
script only marshals bytes; it never decides anything about ZON itself.

Output:
  {
    "source": {"repo":..., "commit":..., "zig":..., "generator":...},
    "cases": [
      {"name":..., "origin":..., "source":..., "valid":true,  "value": <json>},
      {"name":..., "origin":..., "source":..., "valid":false, "error": "..."}
    ]
  }
"""
import json, subprocess, sys

if len(sys.argv) != 6:
    sys.exit(__doc__)
docs_path, oracle, repo, commit, zigver = sys.argv[1:]

docs = json.load(open(docs_path, encoding="utf8"))
if not docs:
    sys.exit("adjudicate.py: no documents to adjudicate")

# Length-prefixed framing: "<byte length>\n<bytes>" per document. Length- rather
# than newline-delimited because ZON documents contain newlines.
blob = b"".join(
    (str(len(b)).encode() + b"\n" + b)
    for b in (d["source"].encode("utf8") for d in docs)
)

proc = subprocess.run([oracle], input=blob, stdout=subprocess.PIPE)
if proc.returncode != 0:
    sys.exit("adjudicate.py: oracle exited %d" % proc.returncode)

lines = proc.stdout.decode("utf8").splitlines()
if len(lines) != len(docs):
    sys.exit(
        "adjudicate.py: oracle produced %d verdicts for %d documents -- refusing "
        "to emit a corpus that is out of step with its adjudication"
        % (len(lines), len(docs))
    )

def _int(s):
    # json.loads maps "-0" to the int 0 and loses the sign, which would turn
    # the oracle's negative zero into a plain zero and wrongly fail a parser
    # that got `-0.0` right. Keep it a float so the sign survives the round
    # trip (json.dump then writes it back as -0.0).
    return float(s) if s == "-0" else int(s)


cases = []
for doc, line in zip(docs, lines):
    verdict = json.loads(line, parse_int=_int)
    case = {"name": doc["name"], "origin": doc["origin"], "source": doc["source"]}
    if verdict["ok"]:
        case["valid"] = True
        case["value"] = verdict["value"]
    else:
        case["valid"] = False
        case["error"] = verdict["error"]
    cases.append(case)

json.dump(
    {
        "source": {
            "repo": repo,
            "commit": commit,
            "zig": zigver,
            "generator": "scripts/fetch-zigzon.sh + test/zigzon/tools/",
            "note": "GENERATED, NEVER COMMITTED. Regenerate with scripts/fetch-zigzon.sh.",
        },
        "cases": cases,
    },
    sys.stdout,
    indent=1,
)
