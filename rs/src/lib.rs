/* Copyright (c) 2025-2026 Richard Rodger, MIT License */

// The engine's error carries a code, position, hint and a formatted
// report, so it is large by design and `Result<_, TabnasError>` trips
// clippy's `result_large_err`. The engine and the jsonic crate allow the
// lint at their own roots for the same reason; boxing here would make
// `parse` return a different shape from `Tabnas::parse`.
#![allow(clippy::result_large_err)]

//! The Zig Object Notation (ZON) grammar plugin for the `tabnas` parsing
//! engine.
//!
//! ZON is the data format of Zig `build.zig.zon` manifests, built on Zig
//! anonymous struct literals:
//!
//! ```zon
//! .{
//!     .name = "example",
//!     .version = "0.0.1",
//!     .dependencies = .{ .foo = .{ .url = "https://..." } },
//!     .paths = .{ "build.zig", "src" },
//! }
//! ```
//!
//! This is a jsonic plugin: it layers on the relaxed-JSON grammar of
//! [`tabnas_jsonic`] and reshapes it into ZON, exactly as the canonical
//! TypeScript plugin in `ts/src/zon.ts` does. It switches the jsonic
//! extensions off (`rule.exclude: jsonic,imp`), remaps the fixed tokens
//! (`.{` opens both structs and tuples, `}` closes both, `=` separates a
//! field from its value), adds five lex matchers for Zig syntax, and
//! prepends the grammar alternates in `zon-grammar.jsonic`, which every
//! runtime embeds.
//!
//! ```
//! let value = tabnas_zon::parse(".{ .name = \"Alice\", .age = 30 }")?;
//! assert_eq!(value.to_string(), r#"{"name":"Alice","age":30}"#);
//! # Ok::<(), tabnas_zon::ZonError>(())
//! ```
//!
//! TypeScript is canonical: `ts/src/zon.ts` defines behaviour and option
//! defaults. The shared fixtures in `test/spec/*.tsv` and the two zig
//! reference corpora are the parity contract across TypeScript, Go and
//! Rust.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tabnas::{
    ActionError, Context, GrammarSetting, GrammarSpec, Plugin, PluginError, Rule, RuleSnapshot,
    Tabnas, Token, Value,
};

mod lex;
mod number;

/// This crate's version. It MUST equal `ts/package.json` "version": the
/// release orchestrator rewrites both, and `tests/version_test.rs` fails
/// the build if they drift. Mirrors `VERSION` in `ts/src/zon.ts` and
/// `const VERSION` in `go/zon.go`.
pub const VERSION: &str = "0.5.6";

/// The README's Rust examples run as doctests, so a stale one fails the
/// gate rather than misleading the reader. Its `toml` and `bash` fences
/// are skipped; rustdoc runs only the `rust` ones.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme_examples {}

/// The error a failed parse produces, re-exported so callers need not
/// depend on the engine crate directly.
pub use tabnas::TabnasError as ZonError;

/// The key of the object an integer literal becomes when no IEEE-754
/// double holds its exact value: `{ "$big": "<decimal digits>" }`.
///
/// The canonical runtime returns such an integer as a `bigint`, the Go
/// port as a `*big.Int`. The engine's [`Value`] has no big-integer
/// variant, so this port keeps the digits instead of rounding them, in
/// the shape the zig reference corpora already use to spell one. A
/// smaller integer, `2^64` included, is a plain [`Value::Number`].
pub const BIG_KEY: &str = "$big";

// --- BEGIN EMBEDDED zon-grammar.jsonic ---
pub const GRAMMAR_TEXT: &str = r##"
# ZON Grammar Definition
# Parses Zig Object Notation (ZON) - a data format based on Zig anonymous
# struct literals.
#
# Example:
#   .{
#       .name = "example",
#       .version = "0.0.1",
#       .deps = .{ .foo = .{ .url = "https://..." } },
#       .paths = .{ "build.zig", "src" },
#   }
#
# The custom zon-dot lex matcher distinguishes struct (map) and tuple (list)
# openings at lex time by peeking ahead:
#   .{  followed by  .ident =    -> emits #OB  (struct / map)
#   .{  otherwise                -> emits #OS  (tuple / list)
# Both close on } which lexes as #CB. The list rules below use #CB (not
# the default #CS) as the list terminator so that the single } character
# closes both struct and tuple literals.
#
# A bare .identifier emits #TX with val = identifier (the leading dot is
# stripped). This token is both a valid KEY (when followed by =) and a
# valid VAL (when used as an enum literal).
#
# The grammar is applied with { rule: { alt: { g: 'zon' } } } so every
# alt below is automatically tagged with the 'zon' group.

{
  rule: val: open: [
    # Empty .{} -> empty list.
    { s: '#OS #CB' b: 2 p: list g: 'list,empty' }
  ]

  rule: list: open: [
    # @array$ allocates the list node (an empty array). Under the new core
    # the list node is no longer auto-seeded: @tabnas/json's @array$ only
    # runs on its own #OS open alts, which ZON replaces here. Without this
    # the elem rule's @elem-bc/replace would push onto an undefined node.
    { s: '#OS #CB' b: 1 a: '@array$' g: 'list,empty' }
    { s: '#OS' p: elem a: '@array$' g: 'list,open' }
  ]
  rule: list: close: [
    { s: '#CB' g: 'list,close' }
  ]

  rule: elem: close: [
    { s: '#CA #CB' b: 1 g: 'elem,trailing,comma' }
    { s: '#CA' r: elem g: 'elem,next,comma' }
    { s: '#CB' b: 1 g: 'elem,end' }
  ]

  rule: pair: close: [
    { s: '#CA #CB' b: 1 g: 'pair,trailing,comma' }
    { s: '#CA' r: pair g: 'pair,next,comma' }
    { s: '#CB' b: 1 g: 'pair,end' }
  ]
}
"##;
// --- END EMBEDDED zon-grammar.jsonic ---

/// The name the plugin registers under, and so the namespace of its
/// options on the instance.
const PLUGIN_NAME: &str = "zon";

/// The decoration that marks an instance the plugin has already
/// configured, so a re-run cannot prepend the alternates twice. The Go
/// port's `zon-init` guard.
const INIT_MARK: &str = "zon-init";

/// The duplicate-field guard. `/prepend`, so it runs before jsonic's own
/// `@pair-bc`, which performs the assignment (last one wins) and would
/// hide the collision by `@pair-ac`.
const PAIR_BC: &str = "@pair-bc/prepend";

/// The enum-tag rewrap. jsonic takes the `val` before-close phase with
/// `/replace`, which suppresses any `/prepend` on it, so the rewrap runs
/// after close instead, as it does in TypeScript. Here it is `/prepend`
/// rather than the bare name TypeScript uses: the engine wires a bare
/// `@val-ac` only when no `@val-ac/append` is registered, and jsonic
/// registers one. Order within the phase does not matter, because
/// jsonic's after-close leaves an enum token's node alone.
const VAL_AC: &str = "@val-ac/prepend";

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Plugin options. [`Default`] is the canonical `Zon.defaults`: character
/// literals as one-character strings, enum literals as bare strings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ZonOptions {
    /// Parse Zig character literals (`'x'`) as numeric code points rather
    /// than as one-character strings.
    pub char_as_number: bool,

    /// When set, wrap an enum literal used as a value (`.foo`) in
    /// `{ "<enumTag>": "foo" }` instead of producing the bare string
    /// `"foo"`. An empty string means unset, as in the Go port.
    pub enum_tag: Option<String>,
}

impl ZonOptions {
    /// The options as the engine's plugin option bag.
    pub fn to_value(&self) -> Value {
        Value::from_json(&serde_json::to_value(self).expect("the options serialise"))
    }

    /// The options read from a plugin option bag. A missing field is its
    /// default and an unknown field is ignored, as the canonical plugin
    /// reads `options.charAsNumber` and `options.enumTag` and nothing
    /// else.
    pub fn from_value(value: &Value) -> Self {
        serde_json::from_value(value.to_json()).unwrap_or_default()
    }

    fn tag(&self) -> Option<String> {
        self.enum_tag.clone().filter(|tag| !tag.is_empty())
    }
}

// ---------------------------------------------------------------------------
// The grammar document
// ---------------------------------------------------------------------------

/// The jsonic option overrides, as the serialized `options` block of the
/// grammar document, so the plugin applies them atomically alongside its
/// rule alternates: the `grammarDef.options` of the canonical plugin.
fn options_document() -> serde_json::Value {
    json!({
        "rule": {
            // Remove the jsonic extensions (implicit maps and lists,
            // top-level commas, path dives). ZON uses explicit struct
            // literals only.
            "exclude": "jsonic,imp",
            "start": "val",
        },
        "fixed": {
            "token": {
                // Bare `{`, `[`, `]` are not valid in ZON. Struct opening
                // is `.{`, which the zonDot lex matcher handles.
                "#OB": null,
                "#OS": null,
                "#CS": null,
                // `=` replaces `:` as the key/value separator.
                "#CL": "=",
            },
        },
        "tokenSet": {
            // ZON field names are identifiers only.
            "KEY": ["#TX"],
        },
        "string": {
            "chars": "\"",
            "multiChars": "",
            // Zig-flavoured escape sequences.
            "escape": {
                "n": "\n",
                "r": "\r",
                "t": "\t",
                "\\": "\\",
                "\"": "\"",
                "'": "'",
            },
            "allowUnknown": false,
        },
        // The relaxed number lexer accepts `+1`, `.5`, `5.`, `0123`,
        // `1__0` and friends, none of which are ZON. The zonNumber matcher
        // implements Zig's numeric literal grammar exactly instead.
        "number": {
            "lex": false,
        },
        "error": {
            "zon_number": "invalid ZON number literal: {src}",
            "zon_ident": "invalid ZON identifier: {src}",
            "zon_char": "invalid ZON character literal: {src}",
            "zon_doc_comment": "doc comments are not allowed in ZON: {src}",
            "zon_dup_field": "duplicate struct field name: {src}",
        },
        // Only `//` line comments in ZON.
        "comment": {
            "lex": true,
            "def": {
                "hash": { "lex": false },
                "slash": { "line": true, "start": "//", "lex": true, "eatline": false },
                "multi": { "lex": false },
            },
        },
        // `true`, `false` and `null` still lex with the text matcher off,
        // because the value matcher runs on its own. The engine's default
        // definitions are exactly these three; `true` and `false` are
        // restated as the canonical plugin states them, and `null` is
        // left to the default because a serialized `"val": null` reads as
        // "no value" rather than as the null value.
        "value": {
            "lex": true,
            "def": {
                "true": { "val": true },
                "false": { "val": false },
            },
        },
        // The text matcher is off: identifiers only ever appear as
        // `.ident` / `.@"..."` and are produced by zonDot.
        "text": {
            "lex": false,
        },
        "lex": {
            "match": {
                "zonDot": { "order": 1e5, "make": lex::DOT },
                "zonMultiString": { "order": 1.1e5, "make": lex::MULTI_STRING },
                "zonChar": { "order": 1.2e5, "make": lex::CHAR },
                "zonNumber": { "order": 1.3e5, "make": lex::NUMBER },
                // Must out-order the comment matcher so `//!` and `///`
                // are rejected instead of being eaten as line comments.
                "zonDocComment": { "order": 1.4e5, "make": lex::DOC_COMMENT },
            },
        },
    })
}

/// The grammar document: [`GRAMMAR_TEXT`] parsed with jsonic, as the
/// canonical plugin parses it (`new Tabnas().use(jsonic).parse(grammarText)`),
/// with the option overrides attached.
///
/// The text is parsed once per process: it is a fixed literal, so every
/// instance gets the same rule table.
fn grammar_document() -> Result<serde_json::Value, PluginError> {
    static RULES: OnceLock<Result<serde_json::Value, String>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        let parsed = tabnas_jsonic::parse(GRAMMAR_TEXT)
            .map_err(|error| format!("zon: failed to parse grammar text: {error}"))?;
        let document = integral_numbers(parsed.to_json());
        if !document
            .get("rule")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err("zon: grammar text did not parse to a rule table".to_string());
        }
        Ok(document)
    });
    let mut document = rules.clone().map_err(PluginError)?;
    document["options"] = options_document();
    Ok(document)
}

/// jsonic's numbers are doubles, so `b: 2` arrives as `2.0`; the alternate
/// decoder reads an integer field as an integer. Put every whole number
/// back into integer form.
fn integral_numbers(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => match number.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
            _ => serde_json::Value::Number(number),
        },
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(integral_numbers).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, integral_numbers(value)))
                .collect(),
        ),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Lifecycle hooks
// ---------------------------------------------------------------------------

/// Zig rejects a struct literal that repeats a field name. jsonic's own
/// `@pair-bc` performs the assignment (last one wins), so this guard runs
/// before it, and fails the parse by handing back the key's token marked
/// `zon_dup_field`, as the canonical hook returns it.
fn pair_duplicate_guard(
    rule: &mut Rule,
    context: &mut Context,
    _next: Option<&RuleSnapshot>,
    _out: Option<Token>,
) -> Result<Option<Token>, ActionError> {
    if !matches!(rule.u.get("pair"), Some(Value::Bool(true))) {
        return Ok(None);
    }
    let Some(Value::String(key)) = rule.u.get("key").cloned() else {
        return Ok(None);
    };
    let present = match &*rule.node.borrow() {
        Value::Object(fields) => fields.contains_key(&key),
        Value::MapRef(map) => map.value.contains_key(&key),
        _ => false,
    };
    if !present {
        return Ok(None);
    }
    let Some(mut token) = rule.o0().cloned().or_else(|| context.t0().cloned()) else {
        return Ok(None);
    };
    token.bad_with_details("zon_dup_field", [("key".to_string(), Value::String(key))]);
    Ok(Some(token))
}

/// With `enumTag` set, wrap the node an enum literal token produced in
/// `{ [enumTag]: name }`. The token carries the mark the zonDot matcher
/// set, which is what tells `.foo` from the string `"foo"`.
fn enum_rewrap(tag: String) -> impl Fn(&mut Rule, &mut Context) -> Result<(), ActionError> {
    move |rule, _context| {
        if !rule.child_node.is_undefined() || rule.os() == 0 {
            return Ok(());
        }
        let Some(token) = rule.o0() else {
            return Ok(());
        };
        if !matches!(
            token.use_data().get(lex::ENUM_MARK),
            Some(Value::Bool(true))
        ) {
            return Ok(());
        }
        let Value::String(name) = token.val.clone() else {
            return Ok(());
        };
        let mut wrapped = IndexMap::new();
        wrapped.insert(tag.clone(), Value::String(name));
        // A fresh cell: the rule's node is ASSIGNED, never written through
        // the cell it may share with its parent.
        rule.node = Rc::new(RefCell::new(Value::object(wrapped)));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

/// Install the ZON grammar on a jsonic-enabled instance, with typed
/// options: the plugin function itself.
///
/// The instance must already carry the relaxed-JSON grammar, from
/// [`tabnas_jsonic::make`] or [`tabnas_jsonic::jsonic`]; ZON reshapes
/// jsonic's `val` / `map` / `list` / `pair` / `elem` rules rather than
/// declaring its own. A second call on the same instance is a no-op, as
/// in the Go port, so a plugin re-run cannot double the alternates.
///
/// ```
/// let mut parser = tabnas_jsonic::make();
/// tabnas_zon::zon(&mut parser, &tabnas_zon::ZonOptions::default())?;
/// assert_eq!(parser.parse(".{ 1, 2, 3 }")?.to_string(), "[1,2,3]");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn zon(parser: &mut Tabnas, options: &ZonOptions) -> Result<(), PluginError> {
    if parser.decoration::<bool>(INIT_MARK).is_some() {
        return Ok(());
    }
    // ZON reshapes jsonic's rules rather than declaring its own, so an
    // instance without them (a bare engine) gets a clear refusal now
    // instead of a grammar that can parse nothing later.
    let names = parser.rule_names();
    let missing: Vec<&str> = ["val", "map", "list", "pair", "elem"]
        .into_iter()
        .filter(|rule| !names.iter().any(|name| name.as_str() == *rule))
        .collect();
    if !missing.is_empty() {
        return Err(PluginError(format!(
            "zon: the instance has no {} rule; install the jsonic grammar first",
            missing.join(", ")
        )));
    }

    // Every closure the grammar names, registered before the document
    // that names them is installed.
    parser.state_action_with_next_ref(PAIR_BC, pair_duplicate_guard);
    if let Some(tag) = options.tag() {
        parser.state_action_ref(VAL_AC, enum_rewrap(tag));
    }
    lex::register(parser, options.char_as_number);

    let spec = GrammarSpec::from_value(grammar_document()?)
        .map_err(|error| PluginError(format!("zon: invalid grammar document: {error}")))?;
    // Every alternate is tagged `zon`, so a caller can exclude them with
    // `rule.exclude`.
    parser
        .grammar_with_setting(&spec, &GrammarSetting::groups("zon"))
        .map_err(|error| PluginError(format!("zon: failed to apply grammar: {error}")))?;
    // Marked only once everything above succeeded: a failed install (a
    // bare engine, say) must not turn the next call into a silent no-op.
    parser.decorate(INIT_MARK, true);
    Ok(())
}

/// The plugin form of [`zon`], for [`Tabnas::use_plugin`]. Its defaults
/// are [`ZonOptions::default`], and the option bag a caller passes is
/// merged over them by the engine, as `UseDefaults` merges them in Go.
///
/// ```
/// let mut parser = tabnas_jsonic::make();
/// parser.use_plugin(
///     tabnas_zon::plugin(),
///     Some(tabnas_zon::ZonOptions { char_as_number: true, ..Default::default() }.to_value()),
/// )?;
/// assert_eq!(parser.parse("'A'")?.to_string(), "65");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn plugin() -> Plugin {
    Plugin::new(PLUGIN_NAME, |parser, options| {
        zon(parser, &ZonOptions::from_value(options))
    })
    .with_defaults(ZonOptions::default().to_value())
}

/// Build a ZON parser with the given options: the counterpart of the Go
/// `MakeJsonic(opts)` and of `new Tabnas().use(jsonic).use(Zon, opts)`.
///
/// Reuse the result: building the grammar dominates a parse.
///
/// Infallible by design: the documents are fixed literals, so a failure
/// here is a bug in this crate rather than anything a caller did.
///
/// ```
/// let parser = tabnas_zon::make_with(&tabnas_zon::ZonOptions {
///     enum_tag: Some("$enum".to_string()),
///     ..Default::default()
/// });
/// assert_eq!(
///     parser.parse(".{ .kind = .red }")?.to_string(),
///     r#"{"kind":{"$enum":"red"}}"#
/// );
/// # Ok::<(), tabnas_zon::ZonError>(())
/// ```
pub fn make_with(options: &ZonOptions) -> Tabnas {
    let mut parser = tabnas_jsonic::make();
    parser
        .use_plugin(plugin(), Some(options.to_value()))
        .expect("the zon grammar documents are fixed and valid");
    parser
}

/// Build a ZON parser with the default options.
///
/// ```
/// let parser = tabnas_zon::make();
/// assert_eq!(parser.parse(".{ .a = .{ .b = 1 } }")?.to_string(), r#"{"a":{"b":1}}"#);
/// assert!(parser.parse("{ a = 1 }").is_err());
/// # Ok::<(), tabnas_zon::ZonError>(())
/// ```
pub fn make() -> Tabnas {
    make_with(&ZonOptions::default())
}

/// Parse a ZON source string with the shared default parser.
///
/// The engine is built once, on first use, and reused after that, as the
/// Go `Parse` does through `sync.Once`. Reuse is safe: [`Tabnas::parse`]
/// takes `&self` and builds a fresh parse context per call, and `Tabnas`
/// is `Send + Sync`, so concurrent callers share one installed grammar.
///
/// Use [`make_with`] or [`parse_with`] when the parser needs options.
///
/// ```
/// let value = tabnas_zon::parse(".{ .paths = .{ \"build.zig\", \"src\" } }")?;
/// assert_eq!(value.to_string(), r#"{"paths":["build.zig","src"]}"#);
/// assert_eq!(tabnas_zon::parse("0X2A").unwrap_err().code, "zon_number");
/// # Ok::<(), tabnas_zon::ZonError>(())
/// ```
pub fn parse(src: &str) -> Result<Value, ZonError> {
    static DEFAULT: OnceLock<Tabnas> = OnceLock::new();
    DEFAULT.get_or_init(make).parse(src)
}

/// Parse a ZON source string with the given options, on a dedicated
/// instance built for the call: the Go `Parse(src, opts)`. For many
/// parses with the same options, build one parser with [`make_with`].
///
/// ```
/// let options = tabnas_zon::ZonOptions { char_as_number: true, ..Default::default() };
/// assert_eq!(tabnas_zon::parse_with("'\\n'", &options)?.to_string(), "10");
/// # Ok::<(), tabnas_zon::ZonError>(())
/// ```
pub fn parse_with(src: &str, options: &ZonOptions) -> Result<Value, ZonError> {
    make_with(options).parse(src)
}
