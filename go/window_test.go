// Copyright (c) 2026 Richard Rodger and other contributors, MIT License

package tabnaszon

// window_test.go — a parse from a reader (Tabnas.ParseReader, admin ADR-30
// and ADR-33).
//
// MakeJsonic builds the parser whole, as jsonic.Make() then
// j.UseDefaults(Zon, Defaults, opts) builds it, and declares lex.window on
// it, so that a reader is read a window at a time. The matchers Zon
// installs (zonDot, zonMultiString, zonChar, zonNumber, zonDocComment and
// zonString) read the source only through the lexer's window. These tests
// hold it to that: MakeJsonic declares the window and parses every fixture
// row as that composition does; every row, and every probe of
// test/strictness/inputs.txt under the defaults and under the options the
// zig corpora are graded with, parses the same from a reader, in pieces of
// 1, 3, 64 and 4096 bytes, as from its text; and so do tokens of every kind
// longer than the window, and a long document. zigzon_test.go grades every
// document of the zig corpora from a reader as well as from its text.
//
// Go's window sizes are fixed at 64 KiB (the engine's small-window hook is
// unexported), and before each token the window reaches 64 KiB past the
// cursor, so a token meets the window's edge only where it is longer than
// that: the long tokens and the long document carry such tokens of every
// kind.

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/big"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"testing"

	jsonic "github.com/tabnas/jsonic/go"
	tabnas "github.com/tabnas/parser/go"
	support "github.com/tabnas/support/go"
)

// readerMargin is how far past the cursor the window reaches before each
// token is cut: a token longer than this meets the window's edge.
const readerMargin = 64 << 10

// pieces is a reader that hands its bytes over at most size at a time.
type pieces struct {
	text []byte
	at   int
	size int
}

func (p *pieces) Read(buf []byte) (int, error) {
	if p.at == len(p.text) {
		return 0, io.EOF
	}
	n := min(p.size, len(buf), len(p.text)-p.at)
	copy(buf, p.text[p.at:p.at+n])
	p.at += n
	return n, nil
}

// parsed is what a parse shows: its value, shown with its keys in their
// order and its numbers as they are (a big integer, NaN, an infinity and
// -0 among them), or its error's code, position, row, column and token
// text. An error's excerpt is left out: a parse from a reader shows a long
// line only as far back as the window reaches (tabnas/parser
// DIVERGENCE.md), and that is the one difference allowed.
type parsed struct {
	value string
	err   string
}

func parsedOf(v any, err error) parsed {
	if err == nil {
		return parsed{value: show(v)}
	}
	var te *tabnas.TabnasError
	if errors.As(err, &te) {
		return parsed{err: fmt.Sprintf("%s at %d %d:%d %q", te.Code, te.Pos, te.Row, te.Col, te.Src)}
	}
	return parsed{err: "not a TabnasError: " + err.Error()}
}

// show writes a parsed value with an object's keys in their order, so that
// two values are the same where their texts are.
func show(v any) string {
	switch t := v.(type) {
	case nil:
		return "null"
	case *jsonic.OrderedMap:
		parts := make([]string, 0, len(t.Keys))
		for _, k := range t.Keys {
			parts = append(parts, strconv.Quote(k)+":"+show(t.Vals[k]))
		}
		return "{" + strings.Join(parts, ",") + "}"
	case map[string]any:
		keys := make([]string, 0, len(t))
		for k := range t {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		parts := make([]string, 0, len(keys))
		for _, k := range keys {
			parts = append(parts, strconv.Quote(k)+":"+show(t[k]))
		}
		return "map{" + strings.Join(parts, ",") + "}"
	case []any:
		parts := make([]string, 0, len(t))
		for _, e := range t {
			parts = append(parts, show(e))
		}
		return "[" + strings.Join(parts, ",") + "]"
	case float64:
		return strconv.FormatFloat(t, 'g', -1, 64)
	case *big.Int:
		return "big " + t.String()
	case string:
		return strconv.Quote(t)
	case bool:
		return strconv.FormatBool(t)
	}
	if jsonic.IsUndefined(v) {
		return "undefined"
	}
	return fmt.Sprintf("%T %#v", v, v)
}

func (o parsed) String() string {
	if o.err != "" {
		return "error " + o.err
	}
	s := "value " + o.value
	if len(s) > 200 {
		s = s[:200] + "..."
	}
	return s
}

// sameFromReader parses text whole and from a reader in pieces of each
// size, and reports where they differ. It returns the number of reader
// parses compared.
func sameFromReader(t *testing.T, j *tabnas.Tabnas, at, text string, sizes []int) int {
	t.Helper()
	whole := parsedOf(j.Parse(text))
	for _, size := range sizes {
		read := parsedOf(j.ParseReader(&pieces{text: []byte(text), size: size}))
		if read != whole {
			short := text
			if len(short) > 80 {
				short = short[:80]
			}
			t.Errorf("%s: %q in pieces of %d:\n  Parse:       %s\n  ParseReader: %s", at, short, size, whole, read)
		}
	}
	return len(sizes)
}

// readerSizes are the pieces a document is handed over in: down to a byte
// for a short one, and from 64 bytes for one over 8 KiB, whose every
// one-byte read the Go window answers with a copy of itself
// (tabnas/parser#321).
func readerSizes(text string) []int {
	if len(text) > 8<<10 {
		return []int{64, 4096, readerMargin + 3}
	}
	return []int{1, 3, 64, 4096}
}

// zonOptions reads a fixture row's options as MakeJsonic takes them.
func zonOptions(t *testing.T, opts map[string]any) ZonOptions {
	t.Helper()
	var o ZonOptions
	for k, v := range opts {
		switch k {
		case "charAsNumber":
			b, ok := v.(bool)
			if !ok {
				t.Fatalf("charAsNumber is %v", v)
			}
			o.CharAsNumber = &b
		case "enumTag":
			if v != nil {
				s, ok := v.(string)
				if !ok {
					t.Fatalf("enumTag is %v", v)
				}
				o.EnumTag = s
			}
		default:
			t.Fatalf("an option MakeJsonic does not take: %s", k)
		}
	}
	return o
}

// fixtureRows calls each with every row of the shared fixtures: where it
// is, its input and its plugin options.
func fixtureRows(t *testing.T, each func(at, input string, opts map[string]any)) {
	t.Helper()
	dir, err := support.FindSpecDir("")
	if err != nil {
		t.Fatal(err)
	}
	specs, err := support.LoadSpecDir(dir, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(specs) == 0 {
		t.Fatalf("no fixtures in %s", dir)
	}
	for _, spec := range specs {
		for _, row := range spec.Rows {
			opts := map[string]any{}
			if raw := row.Named("opts"); raw != "" {
				if err := json.Unmarshal([]byte(raw), &opts); err != nil {
					t.Fatalf("%s: opts: %v", row.Where(), err)
				}
			}
			each(row.Where(), row.UnescNamed("input"), opts)
		}
	}
}

// madeFor builds one parser per set of options: building the grammar costs
// more than a parse.
func madeFor(t *testing.T) func(opts map[string]any) *tabnas.Tabnas {
	built := map[string]*tabnas.Tabnas{}
	return func(opts map[string]any) *tabnas.Tabnas {
		key, _ := json.Marshal(opts)
		j, ok := built[string(key)]
		if !ok {
			j = MakeJsonic(zonOptions(t, opts))
			built[string(key)] = j
		}
		return j
	}
}

// probes are the probes of test/strictness/inputs.txt, the sources of the
// strictness corpus, decoded as scripts/fetch-zigzon.sh decodes them: \n,
// \r, \t and \\, with blank lines and comment lines skipped.
func probes(t *testing.T) []string {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "test", "strictness", "inputs.txt"))
	if err != nil {
		t.Fatal(err)
	}
	decode := strings.NewReplacer(`\n`, "\n", `\r`, "\r", `\t`, "\t", `\\`, `\`)
	var out []string
	for _, line := range strings.Split(string(raw), "\n") {
		if strings.TrimSpace(line) == "" || strings.HasPrefix(line, "#") {
			continue
		}
		out = append(out, decode.Replace(line))
	}
	if len(out) < 160 {
		t.Fatalf("only %d probes", len(out))
	}
	return out
}

func zigOptions() ZonOptions {
	yes := true
	return ZonOptions{CharAsNumber: &yes, EnumTag: "$enum"}
}

// TestMakeJsonicDeclaresTheWindow: MakeJsonic declares lex.window; the
// plugin alone, which a host may install beneath matchers of its own,
// declares nothing.
func TestMakeJsonicDeclaresTheWindow(t *testing.T) {
	if !MakeJsonic().Config().Window {
		t.Error("MakeJsonic does not declare lex.window")
	}
	if !MakeJsonic(zigOptions()).Config().Window {
		t.Error("MakeJsonic with options does not declare lex.window")
	}
	j := jsonic.Make()
	if err := j.UseDefaults(Zon, Defaults); err != nil {
		t.Fatal(err)
	}
	if j.Config().Window {
		t.Error("the Zon plugin declares lex.window")
	}
}

// TestMakeJsonicParsesAsTheComposition: every fixture row parses through
// MakeJsonic as through jsonic.Make() then j.UseDefaults(Zon, Defaults,
// opts), which dmog composed.
func TestMakeJsonicParsesAsTheComposition(t *testing.T) {
	n := 0
	made := madeFor(t)
	fixtureRows(t, func(at, input string, opts map[string]any) {
		j := jsonic.Make()
		if err := j.UseDefaults(Zon, Defaults, opts); err != nil {
			t.Fatal(err)
		}
		composed := parsedOf(j.Parse(input))
		got := parsedOf(made(opts).Parse(input))
		if got != composed {
			t.Errorf("%s:\n  MakeJsonic: %s\n  composed:   %s", at, got, composed)
		}
		n++
	})
	if n < 190 {
		t.Errorf("only %d rows", n)
	}
}

// TestFixturesFromAReader: every fixture row parses the same from a reader
// as from its text, under MakeJsonic with the row's options.
func TestFixturesFromAReader(t *testing.T) {
	compared := 0
	made := madeFor(t)
	fixtureRows(t, func(at, input string, opts map[string]any) {
		compared += sameFromReader(t, made(opts), at, input, readerSizes(input))
	})
	t.Logf("reader fixtures: %d reader parses (100%%)", compared)
	if compared < 760 {
		t.Errorf("only %d reader parses", compared)
	}
}

// TestStrictnessProbesFromAReader: every probe of the strictness corpus
// parses the same from a reader as from its text, under the defaults and
// under the options the zig corpora are graded with.
func TestStrictnessProbesFromAReader(t *testing.T) {
	compared := 0
	for _, opts := range []ZonOptions{{}, zigOptions()} {
		j := MakeJsonic(opts)
		for _, probe := range probes(t) {
			compared += sameFromReader(t, j, "probe", probe, readerSizes(probe))
		}
	}
	t.Logf("reader probes: %d reader parses (100%%)", compared)
	if compared < 1280 {
		t.Errorf("only %d reader parses", compared)
	}
}

// longTokens are documents each holding one token of every kind the
// matchers cut, long enough to meet the window's edge, well formed and not.
func longTokens(long int) []string {
	r := strings.Repeat
	return []string{
		`.{ .k = "` + r("é", long/2) + `\u{1F600}", .z = 1 }`,
		`.{ .k = "\u{` + r("0", long) + `41}", .z = 1 }`,
		`.{ .k = "` + r("q", long) + `\q", .z = 1 }`,
		`.{ .k = "` + r("u", long),
		`.{ .k = "` + r("x", long) + `\x4`,
		".{ .k = \\\\" + r("m", long) + "\n" + r(" ", long) + "\\\\end\n, .z = 1 }",
		".{ .k = \\\\" + r("m", long) + "\r\n\\\\end",
		`.{ .k = ` + r("1", long) + `.5, .z = 1 }`,
		`.{ .k = ` + r("1_0", long/3) + `.5e-3, .z = 1 }`,
		`.{ .k = ` + r("1", long) + `x, .z = 1 }`,
		`.{ .k = -` + r(" ", long) + `inf, .z = 1 }`,
		`.{ .` + r("a", long) + ` = 1, .z = 1 }`,
		`.{ .k = .` + r(" ", long) + `b, .z = 1 }`,
		".{ .k = ." + r("\n// c", long/5) + "\n{ ." + r(" ", long) + "k = 1 } }",
		`.{ .k = .@"` + r("w", long) + `", .z = 1 }`,
		`.{ .k = '\u{` + r("0", long) + `1F600}', .z = 1 }`,
		`.{ .k = '\u{` + r("0", long) + `110000}', .z = 1 }`,
		`.{ .k = '\u{` + r("0", long) + `41', .z = 1 }`,
		`.{ ` + r(" ", long) + `//! doc` + "\n}",
	}
}

// TestLongTokensFromAReader: a token of every kind longer than the window
// parses the same from a reader as from its text, each matcher reading the
// window on as it cuts it.
func TestLongTokensFromAReader(t *testing.T) {
	docs := longTokens(readerMargin + 2<<10)
	j := MakeJsonic()
	for n, doc := range docs {
		sameFromReader(t, j, fmt.Sprintf("long token %d", n), doc, []int{4096, readerMargin + 3})
		t.Logf("reader long tokens: %d of %d (%d%%)", n+1, len(docs), (n+1)*100/len(docs))
	}
}

// edgeShapes are tokens, each a head, a run and a tail, which
// TestTokensMeetTheWindowEdgeAtEveryPlace places alone at the start of a
// document, with the run as long as puts the window's edge at every place
// of the tail and just around it.
var edgeShapes = []struct{ head, run, tail string }{
	// zonString: escapes and faults, and a source ending inside.
	{`"`, "a", `\x41\u{1F600}\n€"`},
	{`"`, "a", `\x4"`},
	{`"`, "a", `\u{D800}"`},
	{`"`, "a", `\q"`},
	{`"`, "a", `\u{41"`},
	{`"`, "a", `\`},
	// zonMultiString: a line end, and the next line's continuation.
	{`\\`, "a", "\r\n  \\\\b"},
	{`\\`, "a", "\n\n\\\\b"},
	// zonChar: a code point after leading zeros, and faults.
	{`'\u{`, "0", `1F600}'`},
	{`'\u{`, "0", `110000}'`},
	{`'\u{`, "0", `41'`},
	// zonNumber: a fraction and exponent, the keywords after a sign, and
	// faults.
	{``, "1", `.5e-3`},
	{`-`, " ", `inf`},
	{`-`, " ", `nan`},
	{`0x`, "f", `.8p3`},
	{``, "1", `_`},
	{``, "1", `.2.3`},
	// zonDot: a name, the struct opener's look ahead, a quoted name, and
	// faults.
	{`.`, " ", `abc`},
	{`.{`, " ", `.@"a" = 1 }`},
	{`.{`, " ", `.a }`},
	{`.@"`, "a", `\x41"`},
	{`.`, " ", `@""`},
	{`.`, "\n", "// c\n{ .a = 1 }"},
}

// TestTokensMeetTheWindowEdgeAtEveryPlace: a token whose run puts the
// window's edge, 64 KiB past the token's start, at every place of its tail
// parses the same from a reader handed over 4096 bytes at a time, which
// reads the window to exactly 64 KiB, as from its text.
func TestTokensMeetTheWindowEdgeAtEveryPlace(t *testing.T) {
	j := MakeJsonic()
	docs := 0
	for n, shape := range edgeShapes {
		from := readerMargin - len(shape.head) - len(shape.tail) - 2
		for p := from; p <= readerMargin-len(shape.head)+2; p++ {
			doc := shape.head + strings.Repeat(shape.run, p) + shape.tail
			sameFromReader(t, j, fmt.Sprintf("shape %q, run %d", shape.head+shape.run+shape.tail, p), doc, []int{4096})
			docs++
		}
		t.Logf("reader edge shapes: %d of %d (%d%%)", n+1, len(edgeShapes), (n+1)*100/len(edgeShapes))
	}
	if docs < 240 {
		t.Errorf("only %d documents", docs)
	}
}

// longDocument is a document of over a megabyte holding fields of every
// kind, short, and tokens of every kind longer than the window. ending
// closes it.
func longDocument(long int, ending string) string {
	r := strings.Repeat
	var b strings.Builder
	b.WriteString(".{\n")
	for i := 0; i < 1500; i++ {
		fmt.Fprintf(&b, "  .f%d = .{ .s = \"s\\x41\\u{1F600}%d\", .c = '\\u{41}', .n = %d, .x = 0x1.8p%d, "+
			".e = .tag%d, .q = .@\"q %d\", .m = \\\\line %d\n    \\\\more\n  , "+
			".t = .{ 1, -2.5, inf } }, // note %d\n", i, i, i*1_000_003, i%7, i, i, i, i)
		if i == 500 || i == 1000 {
			fmt.Fprintf(&b, "  .ls%d = \"%s\\u{%s41}\",\n", i, r("é", long/2), r("0", long))
			fmt.Fprintf(&b, "  .lm%d = \\\\%s\n%s\\\\end\n  ,\n", i, r("m", long), r(" ", long))
			fmt.Fprintf(&b, "  .ln%d = %s.5e-3,\n", i, r("1_0", long/3))
			fmt.Fprintf(&b, "  .long_name_%s%d = .%sb,\n", r("a", long), i, r(" ", long))
			fmt.Fprintf(&b, "  .lc%d = '\\u{%s1F600}',\n", i, r("0", long))
			fmt.Fprintf(&b, "  .lo%d = .%s\n{ .%sk = 1 },\n", i, r("\n// c", long/5), r(" ", long))
		}
	}
	return b.String() + ending
}

// TestLongDocumentFromAReader: the long document parses the same from a
// reader as from its text, and so does the same document with an error at
// its end, each kind the matchers raise, which a parse from a reader places
// where the whole text does after the window has moved on many times.
func TestLongDocumentFromAReader(t *testing.T) {
	long := readerMargin + 2<<10
	text := longDocument(long, "}\n")
	if len(text) < 1<<20 {
		t.Fatalf("the document is %d bytes", len(text))
	}
	j := MakeJsonic()
	if _, err := j.Parse(text); err != nil {
		t.Fatalf("the long document does not parse: %v", err)
	}
	sameFromReader(t, j, "the long document", text, []int{64, 4096, readerMargin + 3})
	t.Logf("reader long document: 1 of 8 (12%%), %d bytes", len(text))
	endings := []struct{ code, ending string }{
		{"unterminated_string", "  .bad = \"" + strings.Repeat("u", long)},
		{"zon_number", "  .bad = 0X12,\n}"},
		{"zon_char", "  .bad = '\\u{" + strings.Repeat("0", long) + "110000}',\n}"},
		{"zon_ident", "  .bad = .@\"\",\n}"},
		{"zon_dup_field", "  .f0 = 1,\n}"},
		{"zon_doc_comment", "  //! doc\n}"},
		{"invalid_unicode", "  .bad = \"" + strings.Repeat("v", long) + "\\u{D800}\",\n}"},
	}
	for n, e := range endings {
		broken := longDocument(long, e.ending)
		_, err := j.Parse(broken)
		var te *tabnas.TabnasError
		if !errors.As(err, &te) || te.Code != e.code {
			t.Fatalf("ending %d is not %s: %v", n, e.code, err)
		}
		sameFromReader(t, j, fmt.Sprintf("the long document with ending %d", n), broken, []int{4096, readerMargin + 3})
		t.Logf("reader long document: %d of 8 (%d%%)", n+2, (n+2)*100/8)
	}
}
