# Phase 2 report: families, GEDCOM, generator (stops at Gate B)

## Done
- `family-families`: `derive_families(&Graph)`; 22 table-driven cases, 400 random-tree invariant checks, 20k-person perf test.
- `family-gedcom`: parser (BOM, CR/LF/CRLF, CONC/CONT, `@@`, size/line limits, UTF-16 and non-UTF-8 rejected with a line number), importer (`ImportPlan`, then all-or-nothing apply, as verified or as a pending proposal), exporter (5.5.1 and 7.0, PEDI), round-trip stable.
- Synthetic generator (`gen_gedcom [people] [seed]`): deterministic, with adoptions, fosters, remarriages, cousin marriages, Persian names.
- Store: `Store::atomic`, `insert_proposal`, `all_people`, `all_relationships`, `Graph::from_parts`.
- Performance fix: the cycle check used the wrong index and scanned every parent link per insert. An `INDEXED BY idx_rel_b` hint made 5k links go from 6.8 s to 0.5 s. The 10k-person import now applies in about 11 s in a debug build (about 2 s in release is expected; measure with `cargo test --release`).
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` are clean.

## Gate B: please answer
Rules and cases: `docs/gate-b-cases.md`. Mock-up: `docs/gate-b-mockup.png`.
Decisions you can change:
1. Merge rule counts any recorded partnership, even ended ones.
2. A partner who is not a linked parent is not pulled into the column.
3. Three-parent columns get a warning and are never merged.
4. A GEDCOM FAM with two spouses and no MARR/DIV imports as partnership `?`.
5. `SEX X` is exported in both GEDCOM versions.
6. Notes on parent-child links are not exported.

## Not verified
- The original sample `.ged` files were not available here. Put them in `crates/gedcom/tests/data/` and the parity test picks them up.
- Windows build is still untested.
