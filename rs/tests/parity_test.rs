// Cross-runtime conformance, driven by the shared `test/spec/*.tsv`
// fixtures at the repo root (see ../../test/AGENTS.md).
//
// The fixture loader, the escape codec, the `ERROR:<code>` contract and
// the row loop all come from tabnas_support, whose TypeScript and Go
// halves `ts/test/parity.test.ts` and `go/parity_test.go` use to run the
// SAME files, so the three implementations cannot drift without one of
// them going red, and neither can the loaders.
//
// What is left here is only what is specific to zon: how to build the
// parser for a row's options.

mod common;

use tabnas_support::Runner;
use tabnas_zon::make_with;

use common::{row_options, spec_dir, to_failure, to_value};

/// Every fixture in the spec directory. `dir` discovers the files by
/// listing, so adding a .tsv runs it in every runtime without touching
/// any runner, and an empty directory fails rather than passing.
#[test]
fn spec() {
    // A fresh parser per row: the `opts` column is per-case, and plugin
    // options must not leak from one row into the next.
    let runner = Runner::new_with_row(|input, row| {
        let options = row_options(row.named("opts"))?;
        make_with(&options)
            .parse(input)
            .map(|value| to_value(&value))
            .map_err(to_failure)
    });
    runner.dir(spec_dir());
}
