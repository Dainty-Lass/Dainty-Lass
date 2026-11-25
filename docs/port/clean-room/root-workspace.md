# Clean-room record: the root workspace files (RD27)

The root `Cargo.toml`'s workspace layout, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml` and
`.cargo/config.toml` began as copies of Dauntless Revived's.

## Method

1. **Spec.** [`../spec-root-workspace.md`](../spec-root-workspace.md) states what each file must do as facts: members
   and excludes, inherited fields, lints, the release profile, every `[workspace.dependencies]` entry with its
   version and feature set, the cargo config, the toolchain pin and the deny policy, each with the decision row
   behind it. No text, order or comment from the earlier files is carried.
2. **Rewrite.** Each file was written from the spec alone in this project's own layout: in-tree crates and then
   third-party crates in one alphabetical list (no purpose banners), feature lists sorted, a reason only where an
   entry would otherwise surprise, the licence allowlist alphabetical, deny sections in the order advisories,
   licences, bans, sources.
3. **Check.** Behaviour compared against the files they replace (below).

## Similarity check

- **TOML values.** Each old and new file was parsed with `tomllib` and compared as data. The only differences are
  the order of sets (feature lists, `exclude`, the licence allowlist, toolchain components) and the wording of the
  three advisory `reason` strings. Every key and value is otherwise the same: no version, feature, lint, flag or
  URL moved.
- **Cargo.** `Cargo.lock` is byte-identical. `cargo metadata --locked` gives the same packages, workspace members
  and resolve graph; its JSON differs only in the order of each dependency's `features` array, which follows the
  manifest's spelling and is a set to cargo.
- **Tools.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the root
  tests and `cargo deny check` (advisories, bans, licences, sources ok) pass as before, and `restricted/` still
  checks against the root crates by path (`cargo check --workspace`; its `--locked` run fails on `plan` with or
  without this change, because `restricted/Cargo.lock` predates the console-subscriber removal from its graph).
- **Text.** What the old and new files share is values a spec has to name (crate names, versions, feature names,
  lint names, SPDX identifiers, advisory IDs, SQLite flags, URLs) and the TOML keys cargo, clippy, rustfmt, rustup
  and cargo-deny define. No comment or line arrangement is reused.

## Since

RB15 (2026-11-24) deleted restricted's workspace: `exclude` lost `restricted`, the `server-controller` alias (its last
user was restricted's launcher) went, and the comments that cited restricted/ were reworded. No value moved otherwise.
