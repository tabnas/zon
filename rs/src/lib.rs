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
//! field from its value), adds six lex matchers for Zig syntax, and
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
pub const VERSION: &str = "0.5.8";

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
    ///
    /// Each field is read ON ITS OWN, and by JavaScript truthiness, which
    /// is what `!!options.charAsNumber` and `options.enumTag || null`
    /// mean in `ts/src/zon.ts`. Deserializing the bag as a whole instead
    /// let one ill-typed field discard a well-typed one: the bag
    /// `{"charAsNumber": true, "enumTag": false}` failed to deserialize
    /// at `enumTag` and fell back to the DEFAULTS, so `'A'` parsed as
    /// `"A"` where both other runtimes give `65`.
    ///
    /// The bag is read as the ENGINE value it is, never through
    /// [`Value::to_json`]. That projection turns a non-finite number into
    /// `null`, and `null` is falsy where `Infinity` is not: a
    /// `charAsNumber` of `Infinity` is true in the canonical runtime and
    /// would have read as false here, and an `enumTag` of `Infinity`
    /// names the key `Infinity` there and would have read as unset here.
    ///
    /// The conversion is LOSSLESS for a string, the empty one included,
    /// so that `from_value(&options.to_value())` returns the options it
    /// was given. `tag` applies the canonical `|| null` filter at the
    /// point of use instead.
    pub fn from_value(value: &Value) -> Self {
        let field = |name: &str| match value {
            Value::Object(fields) => fields.get(name),
            Value::MapRef(map) => map.value.get(name),
            _ => None,
        };
        ZonOptions {
            char_as_number: field("charAsNumber").is_some_and(truthy),
            enum_tag: field("enumTag").and_then(tag_of),
        }
    }

    fn tag(&self) -> Option<String> {
        self.enum_tag.clone().filter(|tag| !tag.is_empty())
    }
}

/// JavaScript truthiness, which is how the canonical plugin reads an
/// option bag: `false`, `null`, `undefined`, `0`, `-0`, `NaN` and `""`
/// are false, and every other value, `Infinity` and an empty array or
/// object included, is true.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => *number != 0.0 && !number.is_nan(),
        Value::String(text) => !text.is_empty(),
        Value::Text(text) => !text.string.is_empty(),
        _ => true,
    }
}

/// The `enumTag` option as this port's `Option<String>`.
///
/// A string is kept VERBATIM, an empty one included: the conversion
/// layer is lossless and the semantic filter belongs at the point of
/// use, which is [`ZonOptions::tag`]. Anything else is outside the
/// option's declared `null | string`, and the canonical
/// `options.enumTag || null` has already discarded a falsy one before it
/// can become a computed property key, so a falsy one is unset here too.
fn tag_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Text(text) => Some(text.string.clone()),
        other if truthy(other) => Some(key_of(other)),
        _ => None,
    }
}

/// A truthy `enumTag` as the key it names. The option's type is
/// `null | string`, and a string is itself; the canonical plugin uses
/// whatever else it is given as a computed property key, which
/// stringifies it. A boolean spells itself, and a number spells itself as
/// `Number::toString` does, which is neither Rust's shortest form nor
/// serde_json's: `10000000000000000` is written out in full and `1e21`
/// is not. An ARRAY or an OBJECT, further outside the option's type
/// still, keeps its JSON spelling here rather than taking the
/// `Array.prototype.toString` and `[object Object]` of JavaScript, which
/// `DIVERGENCE.md` records, along with the `null` a non-finite element
/// of one becomes on the way through `to_json`.
fn key_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Text(text) => text.string.clone(),
        Value::Number(number) => number::js_number_to_string(*number),
        Value::Bool(flag) => flag.to_string(),
        // Neither reaches here through `tag_of`, which discards a falsy
        // value first, as `options.enumTag || null` does. They are the
        // spellings JavaScript would use if one ever did.
        Value::Null => "null".to_string(),
        Value::Undefined => "undefined".to_string(),
        // A container, where this port stops matching anyway. Every
        // option number is an engine `f64`, so `1` inside one arrives as
        // `1.0`; the same restoration the grammar document needs puts
        // the integer spelling back.
        other => integral_numbers(other.to_json()).to_string(),
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
            // ZON field names are identifiers (`.ident` or `.@"..."`) only.
            //
            // Mirrors the TS `KEY: ['#TX', null, null, null]`, and the
            // three trailing nulls are LOAD-BEARING. The engine overlays a
            // named token set onto the installed one BY INDEX, so a bare
            // `["#TX"]` overwrites slot 0 and leaves the default `#NR`,
            // `#ST` and `#VL` live behind it -- and `.{ .a = 1, "b" = 2 }`
            // parsed. A `null` member clears its position.
            //
            // It does NOT take effect in this port yet: the engine expands
            // `#KEY` into its members when it installs an alternate, and
            // tabnas-jsonic's `#KEY #CL` alternates are installed before
            // this plugin runs. See DIVERGENCE.md, "A field name that is
            // not a field is accepted in Rust", and the test that pins it.
            "KEY": ["#TX", null, null, null],
        },
        // The engine's string matcher is off: `"..."` is lexed by the
        // zonString matcher with Zig's escape set, which is narrower than
        // the relaxed-JSON one and refuses a surrogate `\u{...}`.
        "string": {
            "lex": false,
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
                "zonString": { "order": 1.5e5, "make": lex::STRING },
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

// ---------------------------------------------------------------------------
// The option overrides, against the canonical plugin
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::options_document;
    use std::fs;
    use std::path::Path;

    /// The canonical plugin's `grammarDef.options` literal, read out of
    /// `ts/src/zon.ts`. It is TypeScript source rather than JSON, so the
    /// body is walked rather than parsed.
    fn canonical_overrides() -> String {
        let source =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../ts/src/zon.ts"))
                .expect("ts/src/zon.ts is readable");
        let at = source
            .find("grammarDef.options = ")
            .expect("ts/src/zon.ts assigns grammarDef.options");
        object_body(&source, at).to_string()
    }

    /// The body of the first object literal at or after `from`, with its
    /// braces stripped. Line comments and single-quoted strings are
    /// skipped so a `{`, `}` or `'` inside one cannot close the literal
    /// early. Nothing in the literals read here uses a template literal,
    /// a regular expression or a double-quoted string.
    fn object_body(src: &str, from: usize) -> &str {
        let bytes = src.as_bytes();
        let mut i = from;
        while i < bytes.len() && bytes[i] != b'{' {
            i += 1;
        }
        assert!(i < bytes.len(), "no object literal after byte {from}");
        let start = i + 1;
        let mut depth = 1usize;
        i = start;
        while i < bytes.len() && 0 < depth {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'\'' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'\'' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        assert_eq!(depth, 0, "unbalanced object literal after byte {from}");
        &src[start..i - 1]
    }

    /// Every key of the object literal whose body this is, in source
    /// order, paired with the byte index just past its colon. A key at
    /// brace depth zero is recorded; anything nested belongs to an inner
    /// literal and is left to a second call on that one.
    fn entries(body: &str) -> Vec<(String, usize)> {
        let bytes = body.as_bytes();
        let mut out = Vec::new();
        let mut word = String::new();
        let mut depth = 0usize;
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'\'' => {
                    let start = i + 1;
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'\'' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                    if 0 == depth {
                        word = body[start..i].to_string();
                    }
                    i += 1;
                }
                b'{' | b'[' | b'(' => {
                    depth += 1;
                    i += 1;
                }
                b'}' | b']' | b')' => {
                    depth -= 1;
                    i += 1;
                }
                b':' if 0 == depth => {
                    assert!(!word.is_empty(), "a value with no key in:\n{body}");
                    out.push((std::mem::take(&mut word), i + 1));
                    i += 1;
                }
                b',' if 0 == depth => {
                    word.clear();
                    i += 1;
                }
                c => {
                    if 0 == depth {
                        if c.is_ascii_alphanumeric() || b'_' == c || b'$' == c {
                            word.push(c as char);
                        } else {
                            word.clear();
                        }
                    }
                    i += 1;
                }
            }
        }
        out
    }

    fn keys(body: &str) -> Vec<String> {
        entries(body).into_iter().map(|(key, _)| key).collect()
    }

    /// The body of the object literal bound to `key`.
    fn field<'a>(body: &'a str, key: &str) -> &'a str {
        let (_, at) = entries(body)
            .into_iter()
            .find(|(name, _)| name == key)
            .unwrap_or_else(|| panic!("the canonical literal has no {key} key"));
        object_body(body, at)
    }

    /// The keys of a `serde_json` object, in insertion order.
    fn ported_keys(value: &serde_json::Value) -> Vec<String> {
        value
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect()
    }

    /// AGENTS.md rule 4 requires the jsonic option overrides to exist in
    /// all three runtimes and stay in step. Nothing measured that: the
    /// two lists were kept aligned by reading them side by side. This
    /// reads the canonical list out of `ts/src/zon.ts` and compares it
    /// with the one this port installs, key by key, so an override added
    /// or renamed there fails here instead of drifting silently.
    #[test]
    fn the_option_override_surface_is_the_canonical_one() {
        let canonical = canonical_overrides();
        let ported = options_document();
        assert_eq!(keys(&canonical), ported_keys(&ported));

        // The three tables AGENTS.md names cell by cell rather than by
        // their presence alone: the error catalogue is the code contract
        // a fixture pins, the fixed-token remap is what makes `.{` the
        // only opener, and the matcher orders are what put zonDot ahead
        // of the fixed-token matcher and zonDocComment ahead of the
        // comment matcher.
        assert_eq!(
            keys(field(&canonical, "error")),
            ported_keys(&ported["error"])
        );
        assert_eq!(
            keys(field(field(&canonical, "fixed"), "token")),
            ported_keys(&ported["fixed"]["token"])
        );
        let matchers = field(field(&canonical, "lex"), "match");
        assert_eq!(keys(matchers), ported_keys(&ported["lex"]["match"]));
        for (name, at) in entries(matchers) {
            let body = object_body(matchers, at);
            let (_, order_at) = entries(body)
                .into_iter()
                .find(|(field, _)| "order" == field)
                .unwrap_or_else(|| panic!("{name} declares no order"));
            let text: String = body[order_at..]
                .chars()
                .skip_while(char::is_ascii_whitespace)
                .take_while(|c| !c.is_whitespace() && ',' != *c)
                .collect();
            let want: f64 = text
                .parse()
                .unwrap_or_else(|_| panic!("{name} has a non-numeric order {text:?}"));
            assert_eq!(
                ported["lex"]["match"][&name]["order"].as_f64(),
                Some(want),
                "{name}"
            );
        }
    }

    /// The two places this port's override document deliberately differs
    /// from the canonical one. Both are asserted rather than left to
    /// prose, so a repair in the engine that removes the reason for one
    /// fails here and the note in `README.md` comes out with it.
    #[test]
    fn the_two_override_differences_are_the_documented_ones() {
        let canonical = canonical_overrides();
        let ported = options_document();

        // `null` is left to the engine's default definition: a
        // serialized `"val": null` reads as "no value" rather than as
        // the null value, so restating it would switch the keyword off.
        // The keyword still lexes, which `test/spec/scalars.tsv` pins.
        assert_eq!(
            keys(field(field(&canonical, "value"), "def")),
            ["true", "false", "null"]
        );
        assert_eq!(ported_keys(&ported["value"]["def"]), ["true", "false"]);

        // The canonical plugin also calls `tn.options` with a
        // `config.modify` hook that hangs human token descriptions off
        // `cfg.tokenDesc`, which `@tabnas/railroad` reads for a diagram
        // legend. This engine's config has no such field, and the Rust
        // railroad crate takes the descriptions from its own
        // `ExtractOptions::token_desc` instead, so there is nothing for
        // the plugin to attach and the override document carries no
        // `config` block.
        let source =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../ts/src/zon.ts"))
                .expect("ts/src/zon.ts is readable");
        assert!(source.contains("'zon-tokendesc'"));
        assert!(source.contains("cfg.tokenDesc"));
        assert_eq!(ported.get("config"), None);
    }
}
