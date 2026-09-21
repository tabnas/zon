# Build, test and publish the TypeScript (ts/), Go (go/) and Rust (rs/)
# implementations. ts/ is canonical; go/ and rs/ track it.
#
# Local build/test resolve the unpublished @tabnas siblings via the
# repo-set go.work + node_modules symlinks (admin/scripts/link.sh).

.PHONY: all build test clean build-ts build-go build-rs test-ts test-go test-rs corpus \
        clean-ts clean-go clean-rs publish-ts publish-go version-rs tags-go reset \
        prose prose-counts

all: build test

# --- Conformance corpora ---
# Generated from the pinned ziglang/zig 0.16.0 release; .gitignore'd, never
# committed. All three runtimes generate them THEMSELVES before grading — the
# ts/ `pretest` hook, go/'s TestMain and rs/'s tests/zigzon_test.rs — so this
# target is only for building them by hand. It is deliberately not a prerequisite of `make test`. When a corpus
# is missing the conformance suites fail; they never skip.
corpus:
	bash scripts/fetch-zigzon.sh

build: build-ts build-go build-rs

test: test-ts test-go test-rs

clean: clean-ts clean-go clean-rs

# --- TypeScript (package in ts/) ---
build-ts:
	cd ts && npm run build

test-ts:
	cd ts && npm test

clean-ts:
	rm -rf ts/dist ts/dist-test

# Publish the TypeScript package at its current package.json version.
publish-ts: test-ts
	cd ts && npm publish --access public

# --- Go (module in go/) ---
build-go:
	cd go && go build ./...

test-go:
	cd go && go test -v ./...

clean-go:
	cd go && go clean

# Publish the Go module: make publish-go V=x.y.z
# Injects V into the Go `VERSION` const, commits, tags go/vX.Y.Z, and
# (when gh is available) creates a GitHub release.
publish-go: test-go
	@test -n "$(V)" || (echo "Usage: make publish-go V=x.y.z" && exit 1)
	sed -i.bak 's/^const VERSION = ".*"/const VERSION = "$(V)"/' go/zon.go
	rm -f go/zon.go.bak
	git add go/zon.go
	git commit -m "go: v$(V)"
	git tag go/v$(V)
	git push origin main go/v$(V)
	@command -v gh >/dev/null 2>&1 && gh release create go/v$(V) --title "go/v$(V)" --notes "Go module release v$(V)" || true

# --- Rust (crate in rs/) ---
build-rs:
	cd rs && cargo build --all-targets

test-rs:
	cd rs && cargo test --all-targets && cargo test --doc
	cd rs && cargo clippy --all-targets --all-features -- -D warnings

clean-rs:
	cd rs && cargo clean

# Set the Rust crate version: make version-rs V=x.y.z
#
# Bumps BOTH Rust version sites, plus the crate's own entry in
# rs/Cargo.lock, which rs/tests/version_test.rs holds to
# ts/package.json. A release that bumps the TS and Go sites and forgets
# these fails that test.
#
# Unlike publish-go it neither commits nor tags. There is nothing to
# release: the crate depends on the engine and the jsonic grammar by path,
# and crates.io does not accept a path dependency, so tabnas-zon is not
# published. Only the constants need to stay in step.
version-rs:
	@test -n "$(V)" || (echo "Usage: make version-rs V=x.y.z" && exit 1)
	sed -i.bak 's/^version = ".*"/version = "$(V)"/' rs/Cargo.toml
	sed -i.bak 's/^pub const VERSION: &str = ".*";/pub const VERSION: \&str = "$(V)";/' rs/src/lib.rs
	rm -f rs/Cargo.toml.bak rs/src/lib.rs.bak
	cd rs && cargo metadata --format-version 1 --offline >/dev/null

# List published Go module tags, newest first.
tags-go:
	git tag -l 'go/v*' --sort=-version:refname

reset:
	cd ts && npm run reset
	cd go && go clean -cache && go build ./... && go test -v ./...

# The prose gate (see docs/STYLE-GUIDE.md). Vale over the reader-facing
# pages, at the levels set in .vale.ini, on the same file list
# ts/test/docs.test.js reads. Requires `vale` on PATH and one
# `vale sync`. Warnings are advisory, errors fail.
prose:
	vale --minAlertLevel=error $$(node ts/scripts/gated-docs.cjs)
	node ts/scripts/vale-counts.cjs

# Re-measure what .vale.ini and the style guide record, after
# a change to the pages or to the rules moves the numbers.
prose-counts:
	node ts/scripts/vale-counts.cjs --write
