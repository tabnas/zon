// Performance regression guards. Mirrors go/perf_test.go
// (TestParseReusesInstance) and ts/test/perf.test.ts.
//
// The convenience `parse()` must reuse one cached instance rather than
// rebuild the (expensive) ZON grammar on every call: building the grammar
// dominates a parse, so a rebuild-per-call `parse()` is many times slower
// than reusing one `make()` instance. The same holds for USAGE: build one
// instance and reuse it, never rebuild per parse.
//
// Both checks are machine-INDEPENDENT: each compares two paths on the SAME
// machine in the SAME run, so a slow CI box cannot make it flaky (both
// sides scale together). There is deliberately NO absolute wall-clock
// budget.

use std::time::Instant;

use tabnas_zon::{make, parse};

const SRC: &str = ".{ .a = 1, .b = \"x\", .c = .{ 1, 2, 3 } }";

#[test]
fn parse_reuses_its_instance() {
    const N: u32 = 3000;

    // Warm both paths so the comparison is steady-state.
    for _ in 0..100 {
        parse(SRC).expect("parses");
    }
    let parser = make();
    for _ in 0..100 {
        parser.parse(SRC).expect("parses");
    }

    let t0 = Instant::now();
    for _ in 0..N {
        parse(SRC).expect("parse error");
    }
    let convenience = t0.elapsed();

    let t1 = Instant::now();
    for _ in 0..N {
        parser.parse(SRC).expect("reuse parse error");
    }
    let reuse = t1.elapsed();

    // A cached parse() is about equal to instance reuse; allow 4x for
    // scheduling noise. A rebuild-per-call parse() is many times slower,
    // so this catches the regression without an absolute budget.
    let ratio = convenience.as_secs_f64() / reuse.as_secs_f64().max(f64::EPSILON);
    assert!(
        convenience <= reuse * 4,
        "parse() appears to rebuild the grammar on every call: {N} parse() calls took {convenience:?} \
         vs {reuse:?} reusing one instance (ratio {ratio:.1}x, limit 4x). Cache a lazy default \
         instance (see parse / OnceLock)."
    );
    println!("[perf] parse()={convenience:?}  reuse={reuse:?}  ratio={ratio:.2}x");
}

#[test]
fn reusing_one_instance_beats_rebuilding_per_parse() {
    const N: u32 = 300;

    let parser = make();
    for _ in 0..100 {
        assert_eq!(
            parser.parse(SRC).expect("parses").to_string(),
            r#"{"a":1,"b":"x","c":[1,2,3]}"#
        );
    }

    let t0 = Instant::now();
    for _ in 0..N {
        parser.parse(SRC).expect("parses");
    }
    let reuse = t0.elapsed();

    // The anti-pattern this guards against: a fresh engine per parse.
    let t1 = Instant::now();
    for _ in 0..N {
        make().parse(SRC).expect("parses");
    }
    let rebuild = t1.elapsed();

    let ratio = rebuild.as_secs_f64() / reuse.as_secs_f64().max(f64::EPSILON);
    assert!(
        rebuild >= reuse * 4,
        "rebuild-per-parse is not dominated by reuse as expected: rebuild={rebuild:?} \
         reuse={reuse:?} (ratio {ratio:.1}x, expected >4x). Building the grammar should \
         dominate; reuse a single instance."
    );
    println!(
        "[perf] reuse(N={N})={reuse:?}  rebuild(N={N})={rebuild:?}  rebuild/reuse={ratio:.1}x"
    );
}
