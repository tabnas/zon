// Copyright (c) 2025 Richard Rodger and other contributors, MIT License

package tabnaszon

// zigzon_test.go — conformance against the ZIG REFERENCE IMPLEMENTATION.
//
// Two corpora, both GENERATED (never committed) by `scripts/fetch-zigzon.sh`
// from the pinned ziglang/zig 0.16.0 release:
//
//	test/zigzon/cases.json      every .zon file and every ZON snippet in the
//	                            zig tree, plus its verdict/value
//	test/strictness/cases.json  locally authored leniency probes, judged by
//	                            the same reference implementation
//
// Every verdict, and every valid document's expected VALUE, comes from
// `std.zig.Ast` + `std.zig.ZonGen` at that pinned version — this repo never
// decides what ZON means. Both halves are exercised: a valid document must
// parse AND produce the reference value (not merely "it did not throw"), and
// an invalid document must be rejected.
//
// The corpora are not bundled (generating them downloads a pinned zig
// toolchain), so these tests skip when they are absent — the same opt-in
// convention @tabnas/toml and @tabnas/xml use for their external suites. The
// measured result is recorded in AGENTS.md; ts/test/zigzon.test.ts runs the
// identical corpora, so the two runtimes cannot drift.

import (
	"encoding/json"
	"math"
	"math/big"
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
		Zig    string `json:"zig"`
		Commit string `json:"commit"`
	} `json:"source"`
	Cases []zigCase `json:"cases"`
}

// The plugin options that put its output in the SAME shape as the oracle's
// canonical encoding, so values compare directly: `enumTag` distinguishes the
// enum literal `.foo` from the string "foo" (the default flat encoding
// cannot), and `charAsNumber` matches ZonGen's char_literal. This is
// representation alignment, not leniency — no input is excused by it.
func zigOpts() map[string]any {
	return map[string]any{"enumTag": "$enum", "charAsNumber": true}
}

// zigCanon puts a parsed value in the oracle's canonical JSON shape. The
// oracle spells the non-JSON numbers "@inf"/"@-inf"/"@nan" and an integer too
// large for an exact float64 as {"$big": "<decimal>"}; do the same here so a
// CORRECT parse is comparable. It never excuses a wrong one.
func zigCanon(v any) any {
	switch t := v.(type) {
	case *big.Int:
		return map[string]any{"$big": t.String()}
	case float64:
		if math.IsNaN(t) {
			return "@nan"
		}
		if math.IsInf(t, 1) {
			return "@inf"
		}
		if math.IsInf(t, -1) {
			return "@-inf"
		}
		return t
	case *jsonic.OrderedMap:
		out := map[string]any{}
		for _, k := range t.Keys {
			out[k] = zigCanon(t.Vals[k])
		}
		return out
	case map[string]any:
		out := map[string]any{}
		for k, val := range t {
			out[k] = zigCanon(val)
		}
		return out
	case []any:
		out := make([]any, len(t))
		for i, val := range t {
			out[i] = zigCanon(val)
		}
		return out
	}
	return v
}

func loadZigCorpus(t *testing.T, path string) *zigCorpus {
	t.Helper()
	body, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	var doc zigCorpus
	if err := json.Unmarshal(body, &doc); err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	return &doc
}

func zigLabel(c zigCase) string {
	one := strings.Join(strings.Fields(c.Source), " ")
	if 60 < len(one) {
		one = one[:57] + "..."
	}
	return c.Origin + " | " + one
}

func runZigCorpus(t *testing.T, path string) {
	doc := loadZigCorpus(t, path)
	if doc == nil {
		t.Skipf("corpus %s not present; generate it with `bash scripts/fetch-zigzon.sh`", path)
	}
	if len(doc.Cases) == 0 {
		t.Fatalf("%s: corpus has no cases", path)
	}

	valid, invalid := 0, 0
	for _, c := range doc.Cases {
		if c.Valid {
			valid++
		} else {
			invalid++
		}
	}
	// If either half ever empties out, the generator has broken and this
	// suite would go green while measuring half of nothing.
	if valid == 0 || invalid == 0 {
		t.Fatalf("%s: corpus needs both valid (%d) and invalid (%d) documents",
			path, valid, invalid)
	}

	j := jsonic.Make()
	if err := j.UseDefaults(Zon, Defaults, zigOpts()); err != nil {
		t.Fatalf("plugin init: %v", err)
	}

	for _, c := range doc.Cases {
		t.Run(zigLabel(c), func(t *testing.T) {
			got, err := j.Parse(c.Source)

			if !c.Valid {
				if err == nil {
					t.Fatalf("accepted a document zig rejects (%s):\n%s", c.Error, c.Source)
				}
				return
			}
			if err != nil {
				t.Fatalf("rejected a document zig accepts:\n%s\n%v", c.Source, err)
			}

			raw, mErr := json.Marshal(zigCanon(got))
			if mErr != nil {
				t.Fatalf("marshal: %v", mErr)
			}
			var gotVal, wantVal any
			if err := json.Unmarshal(raw, &gotVal); err != nil {
				t.Fatalf("unmarshal got: %v", err)
			}
			if err := json.Unmarshal(c.Value, &wantVal); err != nil {
				t.Fatalf("unmarshal want: %v", err)
			}
			if !reflect.DeepEqual(gotVal, wantVal) {
				t.Errorf("value mismatch for %s\n  got  %s\n  want %s",
					c.Origin, raw, c.Value)
			}
		})
	}
}

// TestZigZon runs every ZON document harvested from the zig tree.
func TestZigZon(t *testing.T) {
	runZigCorpus(t, filepath.Join("..", "test", "zigzon", "cases.json"))
}

// TestZigStrictness runs the leniency probes: inputs designed to catch
// relaxed-JSON behaviour leaking through the jsonic layer, with the verdicts
// supplied by the same reference implementation.
func TestZigStrictness(t *testing.T) {
	runZigCorpus(t, filepath.Join("..", "test", "strictness", "cases.json"))
}
