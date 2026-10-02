package tabnaszon

import (
	"os"
	"testing"
)

func TestTranslationParts(t *testing.T) {
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
