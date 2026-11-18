# Phase 3 report: relationship finding (`family-relate`)

## What it does (spec section 8)
- `RelGraph::new(&Graph, include_pending)` builds integer-indexed adjacency lists once per snapshot; rebuild it after a write.
- `find(a, b, &FindOptions)` returns a `Relation`: `kind` (same / blood / connection / none), `by_blood`, `label` and `reverse_label` (a stable `key` plus numbers plus English `text`, so the front end can translate), `generations`, `common_ancestors`, `path` with each step's kind, `cost`, `chain` text, `has_non_birth`, `via_any` (with `all`), `includes_pending`, `waiting_for_review`.
- **Blood** = closest common ancestor over `birth` links only. Names: same person, parent, grandparent, great-, 2nd great-, child, ..., sibling, half-sibling, aunt/uncle, great-aunt, niece/nephew, nth cousin, m times removed, half cousin. Gendered from each person's sex. Half = exactly one shared common ancestor.
- **Otherwise** the cheapest route (bidirectional Dijkstra): parent or child step 1, partner step 1.5 (+0.5 if ended), a non-birth parent-child step costs the same but is flagged (birth wins exact ties). Named: adoptive / foster / step / guardian / sealed parent and child, adoptive / foster / step-sibling, spouse/partner, in-laws (parent, child, sibling, co-sibling), derived step-parent and step-child. Anything else gets a readable chain, with kinds ("adopted child of", "former partner of").
- `unknown` and `sealing` are not blood. The flaw of the prototype (adopted/step/foster counted as blood) is fixed and tested.
- Pending people and links are ignored unless `include_pending`. A pending endpoint in the verified view gives `waiting_for_review`; with pending included, `waiting_for_review` is set when the path touches pending data.

## Verified
- 10 table-driven correctness tests (blood names both ways round, half and full, great-, 2nd cousins, cousin marriage, adoption, foster / guardian / unknown / sealing, step, in-laws, ended partnership, birth wins ties, `via_any`, unrelated, cost limit, pending, symmetry).
- Benchmark on a synthetic 116,149-person, 191,400-link tree (release): adjacency 19 ms; 30,000 random pairs: **p50 0.33 ms, p95 0.63 ms, p99 0.87 ms, max 10 ms** (target: p95 < 50 ms). `cargo run --release -p family-relate --example bench_relate -- 100000 30000`.
- `cargo test` has a guard (p95 < 50 ms on 30k people in debug).
- fmt, clippy `-D warnings` and the whole workspace's tests are clean.

## Decisions and open questions
- "Half" is declared when exactly one common ancestor is recorded. A sibling pair with only one parent recorded for both therefore reads "half-sibling". Say if you want "sibling" in that case instead.
- Route labels name only simple patterns; guardian, sealing and unknown siblings get the chain instead of a name.
- `max_cost` defaults to 40 (as in the prototype).
- The relationship is not yet exposed over HTTP/MCP (Phase 4).
