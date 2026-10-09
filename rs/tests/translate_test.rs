// The translation parts: what the manifest says and what the crate
// embeds are the same files.
//
// A packaged crate holds nothing outside `rs/`, so the crate embeds its
// own copies, `rs/translate/manifest.json` of `tabnas.plugin.json` and
// `rs/translate/render.alc` of the render the manifest names, as
// `manifest_text()` and `render_text()`. The copies are the only texts a
// host sees, so they must be the files: this holds the embedded manifest
// to the repository's, and the render the manifest names, read from the
// repository, to the embedded one, as it would an embed the manifest
// named. Change the file at the root and run `npm run embed` in `ts/`,
// which copies it into `rs/translate/`; this fails until both are the
// same.

mod common;

use std::fs;

use serde_json::Value;

fn translate() -> Value {
    let manifest: Value =
        serde_json::from_str(tabnas_zon::manifest_text()).expect("the manifest is JSON");
    manifest
        .get("translate")
        .cloned()
        .expect("the manifest carries a translate object")
}

#[test]
fn the_manifest_the_crate_embeds_is_the_repositorys() {
    let on_disk = fs::read_to_string(common::repo_root().join("tabnas.plugin.json"))
        .expect("the repository has its manifest");
    assert_eq!(
        on_disk,
        tabnas_zon::manifest_text(),
        "rs/translate/manifest.json is not tabnas.plugin.json: copy the manifest into rs/translate"
    );
}

#[test]
fn the_render_the_manifest_names_is_the_one_the_crate_embeds() {
    let translate = translate();
    let path = translate["render"]
        .as_str()
        .expect("translate.render names a file");
    let on_disk = fs::read_to_string(common::repo_root().join(path))
        .unwrap_or_else(|e| panic!("translate.render names {path}, which cannot be read: {e}"));
    assert_eq!(
        on_disk,
        tabnas_zon::render_text(),
        "translate.render names {path}, and rs/translate/render.alc, which render_text() \
         embeds, is another text: copy the render into rs/translate"
    );
}

/// An embed takes a plain tree into a format's own schema. ZON's events
/// carry a plain tree, so its manifest names none and the crate carries
/// none; a manifest that named one would be held to its file here, as the
/// render is above.
#[test]
fn the_embed_the_manifest_names_is_the_one_the_crate_embeds() {
    let translate = translate();
    let parts = tabnas_zon::translate().expect("ZON carries translation parts");
    let Some(path) = translate.get("embed").and_then(Value::as_str) else {
        assert_eq!(
            parts.embed, None,
            "the manifest names no embed, and the crate carries one"
        );
        return;
    };
    let on_disk = fs::read_to_string(common::repo_root().join(path))
        .unwrap_or_else(|e| panic!("translate.embed names {path}, which cannot be read: {e}"));
    let embed = parts
        .embed
        .unwrap_or_else(|| panic!("translate.embed names {path}, and the crate carries no embed"));
    assert_eq!(embed.entry, "zon-embed");
    assert_eq!(
        embed.source,
        Some(on_disk.as_str()),
        "translate.embed names {path}, and the crate embeds another text: run npm run embed in ts"
    );
}

#[test]
fn the_structural_interface_names_the_render_entry() {
    let parts = tabnas_zon::translate().expect("ZON carries translation parts");
    assert_eq!(parts.manifest, tabnas_zon::manifest_text());
    assert_eq!(parts.lift, None);
    let render = parts.render.expect("ZON carries a render");
    assert_eq!(render.entry, "zon-render");
    assert_eq!(render.source, Some(tabnas_zon::render_text()));
}

/// ZON is read as a tree and written from one. Its events carry the
/// tree already, so there is no lift, and no accessor for one.
#[test]
fn zon_reads_and_writes_a_tree_with_no_lift() {
    let translate = translate();
    assert_eq!(translate["reads"], "tree");
    assert_eq!(translate["writes"], "tree");
    assert_eq!(translate.get("lift"), None);
}

/// The host prints the loss lines verbatim, so each is a sentence.
#[test]
fn the_loss_is_a_list_of_sentences() {
    let translate = translate();
    let loss = translate["loss"]
        .as_array()
        .expect("translate.loss is a list");
    assert!(!loss.is_empty());
    for line in loss {
        let line = line.as_str().expect("each loss line is a string");
        assert!(
            line.starts_with(char::is_uppercase) && line.ends_with('.'),
            "{line:?} is not a sentence"
        );
    }
}

/// A host links the render with its own program and other formats'
/// parts, so every definition is named for ZON, the entry point is
/// `zon-render`, and the file defines no `export` of its own.
#[test]
fn the_render_is_a_library_named_for_zon() {
    let names: Vec<&str> = tabnas_zon::render_text()
        .lines()
        .filter_map(|line| line.strip_prefix("def "))
        .filter_map(|rest| rest.split_whitespace().next())
        .collect();
    assert!(names.contains(&"zon-render"), "{names:?}");
    for name in &names {
        assert!(name.starts_with("zon-"), "{name} is not named for ZON");
    }
}

/// The documents the render writes for the reader's own big integer, an
/// object whose only member is `$big` holding `-123456789012345678901234567890`,
/// at the root and nested, as alchemy runs the render. Each is that
/// integer, which the reader reads back as its big integer: here the
/// `{ "$big": digits }` object, a bigint in TypeScript and a *big.Int in
/// Go. No shared fixture can spell that value (see test/AGENTS.md), so
/// each runtime's translation test reads it; `test/spec/render.tsv` pins
/// the rest of what the render writes for `$big`.
#[test]
fn the_renders_big_integer_reads_back_as_the_readers() {
    let digits = "-123456789012345678901234567890";
    let root = tabnas_zon::parse("-123456789012345678901234567890\n")
        .expect("the document the render writes for a big integer parses");
    assert_eq!(common::json(&root), format!(r#"{{"$big":"{digits}"}}"#));
    let nested = tabnas_zon::parse(
        ".{\n  .@\"n\" = -123456789012345678901234567890,\n  .@\"a\" = .{\n    -123456789012345678901234567890,\n  },\n}\n",
    )
    .expect("the document the render writes for nested big integers parses");
    assert_eq!(
        common::json(&nested),
        format!(r#"{{"n":{{"$big":"{digits}"}},"a":[{{"$big":"{digits}"}}]}}"#)
    );
}
