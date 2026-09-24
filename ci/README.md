# ci/

Staging area for GitHub Actions workflow changes.

This directory exists because session credentials cannot write
`.github/workflows/*` — see admin `DECISIONS.md` ADR-8. To change CI:

1. Put the intended workflow file in `workflows/`.
2. A maintainer promotes it with the admin `rollout/apply-ci-folders.sh`
   script.

## Pending

Nothing.

## Promoted

Both of these were staged here and now run from `.github/workflows/`:

- **`docs.yml`** — the prose gate: Vale over the reader-facing pages at
  the levels set in `.vale.ini`, on the file list
  `ts/scripts/gated-docs.cjs` produces. See `docs/STYLE-GUIDE.md`.

  It needs no sibling checkouts and no secrets, and pins its own Vale
  version. Errors fail the job; warnings go to the run summary as a
  report. `make prose` runs the identical check locally, and the test
  suite runs the other half of the gate (`ts/test/docs.test.js`).

- **`rust.yml`**, the Rust gate: `ci/rust/run.sh` (formatting,
  build, the shared fixtures, the zig reference corpora, doctests, clippy,
  and a lockfile check that exempts only the sibling crates' versions) on
  the MSRV pinned in `rs/Cargo.toml`. It clones `tabnas/parser`,
  `tabnas/json`, `tabnas/jsonic`, `tabnas/support` and `tabnas/debug`
  beside the checkout, because the crate takes them as path dependencies
  and none is published.
  `make test-rs` is the fast local loop; the script is what CI runs.
