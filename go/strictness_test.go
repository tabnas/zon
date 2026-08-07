// Copyright (c) 2025 Richard Rodger and other contributors, MIT License

package tabnaszon

// strictness_test.go — the LENIENCY probe, Go half.
//
// @tabnas/zon is a JSONIC plugin: the Zon plugin alone cannot parse even
// `.{ .a = 1 }`, so there is no stricter standalone mode to fall back to and
// jsonic's relaxed lexer is inherited unconditionally. This suite measures how
// much of that leaks through as accepted-but-not-ZON.
//
// Inputs are locally authored (test/strictness/inputs.txt, tracked); verdicts
// come from the pinned Zig reference implementation via
// scripts/fetch-zigzon.sh (test/strictness/cases.json, generated, never
// committed).
//
// Runs the SAME inputs, verdicts and options as ts/test/strictness.test.ts.
// Never skips. RED on purpose.

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

type probeCase struct {
	Source string          `json:"source"`
	Valid  bool            `json:"valid"`
	Value  json.RawMessage `json:"value"`
	Error  string          `json:"error"`
}

type probeCorpus struct {
	Source struct {
		Zig    string `json:"zig"`
		Commit string `json:"commit"`
	} `json:"source"`
	Cases []probeCase `json:"cases"`
}

const probeMissing = `
strictness probe verdicts not found at: %s

Generate them (idempotent):
  bash scripts/fetch-zigzon.sh

This suite deliberately FAILS rather than skipping.
`

func probeLabel(src string) string {
	one := strings.ReplaceAll(strings.ReplaceAll(src, "\n", `\n`), "\t", `\t`)
	if 60 < len(one) {
		one = one[:57] + "..."
	}
	return one
}

func TestStrictnessProbe(t *testing.T) {
	path := filepath.Join("..", "test", "strictness", "cases.json")
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf(probeMissing, path)
	}

	var corpus probeCorpus
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatalf("probe %s is not valid JSON: %v", path, err)
	}
	if len(corpus.Cases) == 0 {
		t.Fatalf("probe %s has no cases", path)
	}

	var valid, invalid []probeCase
	for _, c := range corpus.Cases {
		if c.Valid {
			valid = append(valid, c)
		} else {
			invalid = append(invalid, c)
		}
	}
	if len(valid) == 0 {
		t.Fatalf("probe has no valid inputs")
	}
	if len(invalid) == 0 {
		t.Fatalf("probe has no invalid inputs")
	}

	validOk, invalidOk := 0, 0

	t.Run("zig-valid inputs parse to the reference value", func(t *testing.T) {
		for _, c := range valid {
			t.Run(probeLabel(c.Source), func(t *testing.T) {
				got, err := zigParse(c.Source)
				if err != nil {
					t.Fatalf("rejected an input zig accepts: %v", err)
				}
				want := zigWant(t, c.Value)
				if g := zigCanon(got); !reflect.DeepEqual(g, want) {
					t.Fatalf("value mismatch\n  got  %#v\n  want %#v", g, want)
				}
				validOk++
			})
		}
	})

	t.Run("zig-invalid inputs are rejected", func(t *testing.T) {
		for _, c := range invalid {
			t.Run(probeLabel(c.Source), func(t *testing.T) {
				got, err := zigParse(c.Source)
				if err == nil {
					t.Fatalf("LENIENCY LEAK: accepted as %#v an input zig rejects (%s)",
						zigCanon(got), c.Error)
				}
				invalidOk++
			})
		}
	})

	pct := func(a, b int) string { return fmt.Sprintf("%.1f%%", 100*float64(a)/float64(b)) }
	fmt.Fprintf(os.Stderr,
		"\n[strictness] Go baseline @ zig %s\n"+
			"[strictness]   valid-accepted   %d/%d (%s)\n"+
			"[strictness]   invalid-rejected %d/%d (%s)\n",
		corpus.Source.Zig,
		validOk, len(valid), pct(validOk, len(valid)),
		invalidOk, len(invalid), pct(invalidOk, len(invalid)))
}
