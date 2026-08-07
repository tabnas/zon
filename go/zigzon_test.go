// Copyright (c) 2025 Richard Rodger and other contributors, MIT License

package tabnaszon

// zigzon_test.go — conformance against the ZIG REFERENCE IMPLEMENTATION.
//
// The corpus is `test/zigzon/cases.json`, GENERATED (never committed) by
// `scripts/fetch-zigzon.sh` from the pinned ziglang/zig 0.16.0 release. Every
// document's verdict, and every valid document's expected value, comes from
// `std.zig.Ast` + `std.zig.ZonGen` at that same pinned version — not from this
// parser. See test/zigzon/README.md.
//
// This is the exact same corpus and the exact same option set that
// `ts/test/zigzon.test.ts` runs, so the two runtimes cannot drift on it
// without one of them going red.
//
// Both halves are exercised: valid documents must parse AND produce the
// reference VALUE; invalid documents must be REJECTED.
//
// THIS SUITE MUST NEVER SKIP. If the corpus is absent it FAILS with
// instructions. A conformance test that quietly does not run is worse than no
// test, because the green tick is a lie.
//
// It is currently RED on purpose — a measuring instrument, not a gate.

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	jsonic "github.com/tabnas/jsonic/go"
)

type zigCase struct {
	Name   string          `json:"name"`
	Origin string          `json:"origin"`
	Source string          `json:"source"`
	Valid  bool            `json:"valid"`
	Value  json.RawMessage `json:"value"`
	Error  string          `json:"error"`
}

type zigCorpus struct {
	Source struct {
		Repo   string `json:"repo"`
		Commit string `json:"commit"`
		Zig    string `json:"zig"`
	} `json:"source"`
	Cases []zigCase `json:"cases"`
}

func zigCasesPath() string {
	return filepath.Join("..", "test", "zigzon", "cases.json")
}

const zigMissing = `
zigzon conformance corpus not found at: %s

Generate it (idempotent, ~1 min, downloads a pinned zig 0.16.0):
  bash scripts/fetch-zigzon.sh

This suite deliberately FAILS rather than skipping. A conformance test that
silently does not run reports a green tick for work it never did.
`

// zigOpts puts the plugin's output in the SAME shape as the oracle's canonical
// encoding so values compare directly:
//
//	enumTag "$enum"   -> `.foo` becomes {"$enum":"foo"}, matching ZonGen's
//	                     enum_literal (and distinguishing it from the string
//	                     "foo", which the default flat encoding cannot do)
//	charAsNumber true -> `'x'` becomes its codepoint, matching char_literal
//
// Representation alignment, not leniency: no input is excused by it. Must stay
// identical to OPTS in ts/test/zigzon.test.ts.
var zigOpts = map[string]any{"enumTag": "$enum", "charAsNumber": true}

// zigCanon puts the ACTUAL value in the oracle's canonical JSON shape. The
// oracle spells non-finite floats "@inf"/"@-inf"/"@nan" because JSON cannot
// carry them; do the same here so a parse that got them RIGHT can pass. This
// only ever makes a correct parse comparable — it never excuses a wrong one.
func zigCanon(v any) any {
	switch x := v.(type) {
	case *jsonic.OrderedMap:
		m := make(map[string]any, x.Len())
		for _, k := range x.Keys {
			m[k] = zigCanon(x.Vals[k])
		}
		return m
	case map[string]any:
		m := make(map[string]any, len(x))
		for k, val := range x {
			m[k] = zigCanon(val)
		}
		return m
	case []any:
		s := make([]any, len(x))
		for i, val := range x {
			s[i] = zigCanon(val)
		}
		return s
	case float64:
		if math.IsNaN(x) {
			return "@nan"
		}
		if math.IsInf(x, 1) {
			return "@inf"
		}
		if math.IsInf(x, -1) {
			return "@-inf"
		}
		return x
	case int:
		return float64(x)
	case int64:
		return float64(x)
	default:
		return v
	}
}

// zigWant decodes the oracle's expected JSON into the same Go shapes zigCanon
// produces (encoding/json gives float64 / map[string]any / []any).
func zigWant(t *testing.T, raw json.RawMessage) any {
	t.Helper()
	var out any
	if err := json.Unmarshal(raw, &out); err != nil {
		t.Fatalf("bad expected JSON %s: %v", raw, err)
	}
	return out
}

func zigLabel(c zigCase) string {
	one := strings.Join(strings.Fields(c.Source), " ")
	if 60 < len(one) {
		one = one[:57] + "..."
	}
	return c.Origin + " | " + one
}

func zigParse(src string) (result any, err error) {
	// A grammar bug that panics must be reported as a failure to reject, not
	// crash the whole run and take the other 221 verdicts with it.
	defer func() {
		if r := recover(); r != nil {
			result = nil
			err = fmt.Errorf("panic: %v", r)
		}
	}()
	j := jsonic.Make()
	if e := j.UseDefaults(Zon, Defaults, zigOpts); e != nil {
		return nil, fmt.Errorf("plugin init: %w", e)
	}
	return j.Parse(src)
}

func TestZigZonCorpus(t *testing.T) {
	path := zigCasesPath()
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf(zigMissing, path)
	}

	var corpus zigCorpus
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatalf("corpus %s is not valid JSON: %v", path, err)
	}
	if len(corpus.Cases) == 0 {
		t.Fatalf("corpus %s has no cases", path)
	}

	var valid, invalid []zigCase
	for _, c := range corpus.Cases {
		if c.Valid {
			valid = append(valid, c)
		} else {
			invalid = append(invalid, c)
		}
	}
	// If either half ever empties out, the generator has broken and this suite
	// would go green while measuring half of nothing.
	if len(valid) == 0 {
		t.Fatalf("corpus has no valid documents")
	}
	if len(invalid) == 0 {
		t.Fatalf("corpus has no invalid documents")
	}

	validOk, invalidOk := 0, 0

	t.Run("valid documents parse to the reference value", func(t *testing.T) {
		for _, c := range valid {
			t.Run(zigLabel(c), func(t *testing.T) {
				got, err := zigParse(c.Source)
				if err != nil {
					t.Fatalf("rejected a document zig accepts: %v\n%s", err, c.Source)
				}
				want := zigWant(t, c.Value)
				if g := zigCanon(got); !reflect.DeepEqual(g, want) {
					t.Fatalf("value mismatch\n  got  %#v\n  want %#v\n  src  %s", g, want, c.Source)
				}
				validOk++
			})
		}
	})

	t.Run("invalid documents are rejected", func(t *testing.T) {
		for _, c := range invalid {
			t.Run(zigLabel(c), func(t *testing.T) {
				if _, err := zigParse(c.Source); err == nil {
					t.Fatalf("accepted a document zig rejects (%s):\n%s", c.Error, c.Source)
				}
				invalidOk++
			})
		}
	})

	pct := func(a, b int) string { return fmt.Sprintf("%.1f%%", 100*float64(a)/float64(b)) }
	fmt.Fprintf(os.Stderr,
		"\n[zigzon] Go baseline @ zig %s (%.12s)\n"+
			"[zigzon]   valid-accepted   %d/%d (%s)\n"+
			"[zigzon]   invalid-rejected %d/%d (%s)\n",
		corpus.Source.Zig, corpus.Source.Commit,
		validOk, len(valid), pct(validOk, len(valid)),
		invalidOk, len(invalid), pct(invalidOk, len(invalid)))
}
