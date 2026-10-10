package tabnaszon

import (
	"encoding/json"
	"errors"
	"io/fs"
	"math/big"
	"os"
	"testing"
)

func TestTranslationParts(t *testing.T) {
	inCheckout(t)
	parts := Translate()
	if parts == nil {
		t.Fatal("Translate returned nil")
	}
	manifest, err := os.ReadFile("../tabnas.plugin.json")
	if err != nil {
		t.Fatal(err)
	}
	if parts.Manifest != string(manifest) {
		t.Fatal("embedded manifest differs from tabnas.plugin.json")
	}
	if parts.Lift != nil {
		t.Fatal("ZON has no lift")
	}
	if parts.Render == nil || parts.Render.Entry != "zon-render" {
		t.Fatalf("render entry is %#v", parts.Render)
	}
	render, err := os.ReadFile("../alchemy/render.alc")
	if err != nil {
		t.Fatal(err)
	}
	if parts.Render.Source != string(render) {
		t.Fatal("embedded render differs from alchemy/render.alc")
	}
}

// An embed takes a plain tree into a format's own schema. ZON's events
// carry a plain tree, so its manifest names none and the package carries
// none; a manifest that named one would be held to its file here, as the
// render is above.
func TestTranslationEmbed(t *testing.T) {
	inCheckout(t)
	manifest, err := os.ReadFile("../tabnas.plugin.json")
	if err != nil {
		t.Fatal(err)
	}
	var spec struct {
		Translate struct {
			Embed *string `json:"embed"`
		} `json:"translate"`
	}
	if err := json.Unmarshal(manifest, &spec); err != nil {
		t.Fatal(err)
	}
	parts := Translate()
	if spec.Translate.Embed == nil {
		if parts.Embed != nil {
			t.Fatalf("the manifest names no embed, and Translate carries %#v", parts.Embed)
		}
		return
	}
	if parts.Embed == nil || parts.Embed.Entry != "zon-embed" {
		t.Fatalf("embed entry is %#v", parts.Embed)
	}
	embed, err := os.ReadFile("../" + *spec.Translate.Embed)
	if err != nil {
		t.Fatal(err)
	}
	if parts.Embed.Source != string(embed) {
		t.Fatalf("embedded embed differs from %s", *spec.Translate.Embed)
	}
}

// The documents the render writes for the reader's own big integer, an
// object whose only member is $big holding -123456789012345678901234567890,
// at the root and nested, as alchemy runs the render. Each is that
// integer, which the reader reads back as a *big.Int here, a bigint in
// TypeScript and the { "$big": digits } object in Rust. No shared fixture
// can spell that value (see ../test/AGENTS.md), so each runtime's
// translation test reads it; test/spec/render.tsv pins the rest of what
// the render writes for $big.
func TestTranslationBigIntegerReadsBack(t *testing.T) {
	want := bigOf(t, "-123456789012345678901234567890")
	if got, ok := parse(t, "-123456789012345678901234567890\n").(*big.Int); !ok || got.Cmp(want) != 0 {
		t.Errorf("the render's big integer at the root reads back as %#v, want *big.Int %v",
			parse(t, "-123456789012345678901234567890\n"), want)
	}
	src := ".{\n  .@\"n\" = -123456789012345678901234567890,\n  .@\"a\" = .{\n    -123456789012345678901234567890,\n  },\n}\n"
	nested, ok := normalize(parse(t, src)).(map[string]any)
	if !ok {
		t.Fatalf("the render's struct reads back as %#v", parse(t, src))
	}
	if got, ok := nested["n"].(*big.Int); !ok || got.Cmp(want) != 0 {
		t.Errorf("the render's big integer in a struct reads back as %#v, want *big.Int %v", nested["n"], want)
	}
	items, ok := nested["a"].([]any)
	if !ok || len(items) != 1 {
		t.Fatalf("the render's tuple reads back as %#v", nested["a"])
	}
	if got, ok := items[0].(*big.Int); !ok || got.Cmp(want) != 0 {
		t.Errorf("the render's big integer in a tuple reads back as %#v, want *big.Int %v", items[0], want)
	}
}

// Every call hands back parts of its own: a caller that changes what it
// was given, the parts or a part they point to, changes nothing the next
// caller reads, and callers on several goroutines share nothing to race on.
func TestTranslateReturnsACopy(t *testing.T) {
	first := Translate()
	want := *Translate().Render
	first.Manifest = ""
	if first.Render != nil {
		first.Render.Entry = ""
		first.Render.Source = ""
	}
	first.Lift, first.Embed, first.Render = nil, nil, nil
	second := Translate()
	if second.Manifest == "" {
		t.Fatal("a change to one call's manifest reached the next call")
	}
	if second.Render == nil || *second.Render != want {
		t.Fatalf("a change to one call's render reached the next call: %#v", second.Render)
	}
}

// inCheckout skips a test that holds the embedded copies to the
// repository's own files when it runs where those files are not, as from
// the module cache, whose zip holds the go/ module alone. In a checkout,
// a missing file still fails the test that reads it.
func inCheckout(t *testing.T) {
	t.Helper()
	if _, err := os.Stat("../ts/package.json"); errors.Is(err, fs.ErrNotExist) {
		t.Skip("not in a checkout of the repository: the module cache holds the go/ module alone")
	}
}
