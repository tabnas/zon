// Composition test: the ZON grammar plugin layered with the tabnas-debug
// introspection plugin, the Rust half of ts/test/debug-model.test.ts.
//
// The TypeScript suite resolves @tabnas/debug dynamically and SKIPS when
// it is absent. Here tabnas-debug is a dev-dependency on the sibling
// checkout, like every other crate this port takes, so the test FAILS to
// build when the checkout is missing rather than reporting green having
// run nothing, which is the rule the rest of this suite lives by.

use tabnas::Tabnas;
use tabnas_debug::{apply, model, DebugOptions};

/// A jsonic instance with zon installed and the debug plugin layered on
/// top, quiet: introspection only, no `USE:` dump and no tracing, which
/// is what `{ print: false, trace: false }` asks for in TypeScript.
fn build() -> Tabnas {
    let mut parser = tabnas_zon::make();
    apply(&mut parser, DebugOptions::quiet()).expect("the debug plugin installs over zon");
    parser
}

#[test]
fn parses_normally_with_the_debug_plugin_installed() {
    let parser = build();
    let value = parser
        .parse(".{ .a = 1, .b = .{ 2, 3 } }")
        .expect("a struct with a nested tuple parses");
    assert_eq!(value.to_json().to_string(), r#"{"a":1.0,"b":[2.0,3.0]}"#);
}

#[test]
fn the_model_is_the_structured_zon_grammar() {
    let parser = build();
    let m = model(&parser);

    // The structured rule set and the entry rule.
    let mut names: Vec<&str> = m.rules.iter().map(|rule| rule.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["elem", "list", "map", "pair", "val"]);
    assert_eq!(m.config.start, "val");
    assert!(
        m.plugins.iter().any(|plugin| plugin.name == "zon"),
        "plugins should list zon: {:?}",
        m.plugins
            .iter()
            .map(|plugin| &plugin.name)
            .collect::<Vec<_>>()
    );

    // `val` is a choice whose open alts push both the map and the list
    // rule: ZON's `.{ ... }` is disambiguated into struct (map) or tuple
    // (list) by what follows the opening brace.
    let val = m
        .rules
        .iter()
        .find(|rule| rule.name == "val")
        .expect("a val rule");
    let pushes = |name: &str| val.open.iter().any(|alt| alt.push.as_deref() == Some(name));
    assert!(pushes("map"), "val should push map");
    assert!(pushes("list"), "val should push list");

    // The rule-reference graph captures the recursive collection
    // structure: map -> pair, list -> elem, and pair / elem each
    // close-replace themselves to iterate over further members.
    let edge = |name: &str| {
        m.graph
            .iter()
            .find(|edges| edges.name == name)
            .unwrap_or_else(|| panic!("an edge entry for {name}"))
    };
    let mut val_pushes = edge("val").open_push.clone();
    val_pushes.sort_unstable();
    assert_eq!(val_pushes, ["list", "map"]);
    assert_eq!(edge("map").open_push, ["pair"]);
    assert_eq!(edge("list").open_push, ["elem"]);
    assert_eq!(edge("pair").close_replace, ["pair"]);
    assert_eq!(edge("elem").close_replace, ["elem"]);
}

#[test]
fn the_grammar_portion_serialises_and_round_trips() {
    // The TypeScript suite asserts that the grammar portion of the model
    // survives `JSON.parse(JSON.stringify(..))`. The Rust model derives
    // `Serialize` only, so the round trip is serialise, parse as generic
    // JSON, re-serialise, and compare: a value that does not survive that
    // (a non-finite number, a map with non-string keys) fails here.
    let parser = build();
    let m = model(&parser);
    let grammar = serde_json::json!({
        "tokens": m.tokens,
        "rules": m.rules,
        "graph": m.graph,
        "config": m.config,
        "abnf": m.abnf,
    });
    let text = serde_json::to_string(&grammar).expect("the grammar portion serialises");
    let back: serde_json::Value = serde_json::from_str(&text).expect("and parses back");
    assert_eq!(back, grammar);
    assert_eq!(
        back["rules"],
        serde_json::to_value(&m.rules).expect("rules serialise")
    );
    assert_eq!(back["config"]["start"], "val");
}
