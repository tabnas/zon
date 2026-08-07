# Agents Guide — test corpora

This directory holds three things:

| Path | What | Committed? |
|---|---|---|
| `spec/*.tsv` | hand-written cross-runtime parity fixtures (below) | yes |
| [`zigzon/`](zigzon/README.md) | conformance corpus from the Zig reference implementation | **tools only** — `cases.json` / `vendor/` are generated and `.gitignore`d |
| [`strictness/`](strictness/README.md) | leniency probe | `inputs.txt` yes; `cases.json` generated |

**Never commit a third-party corpus.** `zigzon/cases.json` is derived from
ziglang/zig and is fetched by `scripts/fetch-zigzon.sh` at a SHA-256-pinned
release. The `.gitignore` rules exist to prevent it being committed; do not
remove them.

Note the division of labour, which is what makes the conformance numbers mean
anything: `spec/*.tsv` records what *this repo* expects, so it can only catch
TS/Go drift. `zigzon/` and `strictness/` are adjudicated by *Zig*, so they can
catch this repo being wrong in both runtimes at once — which, at the current
baseline, it frequently is.

## Shared spec fixtures

`spec/*.tsv` holds the cross-runtime conformance fixtures. Both runtimes
auto-discover and run **every** file in this directory, so a change here
affects TypeScript and Go together — edit with that in mind.

## Format

Tab-separated, one case per line, with a header row naming the columns.
Blank lines are skipped, and so are comment lines — a line starting with
`#` that contains no tab. (A data row always has at least one tab, so a
`#`-leading source such as a C preprocessor directive still works.)

| Column | Meaning |
|---|---|
| `input` | ZON source. Escapes `\n` `\r` `\t` `\\` are decoded. |
| `expected` | A JSON value (the parse result), or `ERROR` / `ERROR:<substring>` for inputs that must fail. |
| `opts` | Optional JSON object of plugin options (empty means defaults). |

`expected` and `opts` are **not** escape-decoded — they are raw JSON, so
JSON's own escape rules apply (`"a\nb"` is a string containing a newline).
To put a literal backslash in `input`, write `\\`.

Results are compared after a JSON round-trip, so key order and the
`OrderedMap` / null-prototype-object representations do not affect the
comparison.

## Who runs what

- TypeScript: `ts/test/parity.test.ts` — reads `../../test/spec` at runtime
  from `dist-test/`, one `describe` per file.
- Go: `go/parity_test.go` — `TestSpec` globs `../test/spec/*.tsv`.

Both discover files by directory listing: adding a `.tsv` here runs it in
both runtimes without touching either runner.

## Rules

- Prefer adding a fixture here over a one-off in-language assertion when a
  case is expressible as input → output. That is what keeps the two
  runtimes honest against each other.
- TypeScript is canonical. If the two runtimes disagree, the TS behaviour is
  the expected value — unless Go has exposed a genuine TS defect, in which
  case fix TS first and pin the corrected behaviour here.
- A new fixture must pass in BOTH runtimes: run `go test ./...` (from `go/`)
  and `npm test` (from `ts/`) before considering it done.
