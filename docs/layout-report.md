# Layout engine report (Gate E)

Crate `family-layout`: pure module (people and columns in; layer per person, row order, column order out). No UI, server or SQL. Graphviz is used only by the development tool `examples/layout_compare.rs`.

## 1. What already existed (checked first)
| Where | What | Used by us |
|---|---|---|
| Our workspace | nothing: `derive_families` builds columns but does not rank or order | n/a, so the whole engine was left to build |
| Original GeneaQuilts `GenerationRank` | network-simplex style ranker | dead code in the shipped flow; not ported |
| Original `QuiltManager.sortLayers` | barycentre sweeps (max 100 iterations) | runs only without Graphviz layers; has the `NaN` comparison bug; not ported |
| Original `LayerRank`, `LayerRank2`, `LayerClusterer`, `VertexOrder` | unused / unfinished | not ported |
| Original, in practice | Graphviz `dot` decides everything (rank and x of every vertex) | this is the reference we are measured against |
| Owner's Node prototype `layout.js` | card-tree layout with barycentre sweeps on partner "units" | different picture (cards, not a matrix); only the sweep idea is reused |

## 2. What was built
1. **Ranking** (`rank.rs`, `simplex.rs`): bipartite graph person -> column -> child, every edge at least one layer. Longest path, then compaction, then **network simplex** (Gansner et al. 1993, written from the paper) per tree of up to 12,000 nodes. Bigger trees keep the compaction result. Cycles in bad data are broken (forced node count is reported), never a crash.
2. **Ordering** (`order.rs`): per tree, depth-first start, barycentre and median sweeps (4 variants, best by the quilt score), then neighbour-swap refinement on the real score within a fixed deterministic work budget (no clock).
3. **Output**: trees one after another, largest first, people alone last; each tree is its own diagonal of blocks. Rows by (tree, layer, position), columns likewise. Ties by earliest possible birth day, then person index.
4. **Hint** (`layout_with_hint`): previous positions in, same order kept for what existed, new people slotted next to their neighbours, local refinement only.

## 3. The fixed scoring function (`score.rs`)
Defined on the finished quilt only, so it is identical for our layout and for Graphviz.
* crossings: a row line (first to last cell of a person) and a column line (first to last cell of a family) that meet where the person is not in that family. Counted by a sweep in O(n log n); checked against a brute-force version on 400 random cases.
* row-line length: sum over people of (last column - first column). Column-line length is reported too.
* "No worse" = crossings and row-line length are each less than or equal to the reference.

Reference: `dot -Tplain` on the same bipartite graph (all columns, including a child's several parent columns), ranks from y, order from x. Two variants: **original** (each layer sorted by (tree, x), as GeneaQuilts does) and **per-tree** (blocks of one tree together, the stronger one). We must beat both.

## 4. Scores (release build; full table in `docs/gate-e-table.txt`)
Format: crossings / row-line length. All 15 real sample files and 3 synthetic ones are **no worse than both references**.

| file | people | ours | dot original | dot per-tree | our time |
|---|---|---|---|---|---|
| Simpsons | 11 | 0 / 3 | 1 / 3 | 1 / 3 | 0.1 ms |
| Kennedy+Family | 69 | 7 / 69 | 41 / 95 | 41 / 95 | 0.9 ms |
| Bible Family Tree | 583 | 95 / 1263 | 1196 / 1669 | 912 / 1373 | 0.12 s |
| mason | 702 | 1450 / 6258 | 4200 / 8183 | 4200 / 8183 | 0.19 s |
| greek-myth | 803 | 6121 / 15605 | 8028 / 17154 | 7853 / 16790 | 0.29 s |
| royal92 | 3010 | 21563 / 55201 | 39168 / 63198 | 39125 / 63147 | 7.0 s |
| Ragusan | 5999 | 424441 / 739764 | 483409 / 785553 | 483409 / 785553 | 15.6 s |
| synthetic 1000 | 1275 | 27891 / 82157 | 37277 / 86618 | 37267 / 86598 | 1.0 s |
| synthetic 3000 | 3862 | 343686 / 824158 | 361461 / 825216 | 361433 / 825020 | 5.7 s |
| synthetic 8000 | 8673 | 1584427 / 3595089 | 1699828 / 3774947 | 1699365 / 3773987 | 7.7 s |

Where it is thin: synthetic 3000 row-line length is only 0.1% better than the per-tree reference. Total edge length of the ranking equals Graphviz's on every file where both were measured (royal92 4717, Ragusan, greek-myth 2116, mason 965).
No gap remains on any file, so nothing to ask you about the gate itself.

## 5. Stability (one person added or edited; share of person pairs whose relative order changed)
| tree | no hint, worst | with previous layout as hint, worst |
|---|---|---|
| 708 people | rows 0.45%, cols 0.77% | rows 0.003%, cols 0.86% |
| 1,588 people | rows 7.6%, cols 7.9% | rows 0.003%, cols 0.54% |
| 5,492 people (12 trees) | rows 0.06%, cols 0.13% | rows 0.001%, cols 0.12% |
20 trials each: add a child, add a spouse with a child, edit a birth date. The 0.1-0.9% column movement with a hint is the swap refinement continuing to improve the old layout, not the edit (it is the same for every trial).
Honest reading: without a hint, a mid-size single tree can reshuffle up to 8% of pairs, because the best of several heuristic starts can flip. With the previous layout kept (as the app can) an edit is local. The test `one_edit_moves_little` asserts under 2% with a hint and under 10% without.

## 6. Benchmark, synthetic tree
| people | derive_families | layout (rank + order) |
|---|---|---|
| 10,756 | 6 ms | 40 ms |
| 113,476 (54,119 columns, 11 trees) | 114 ms | **284 ms** |
At that size network simplex and refinement are skipped (size caps), so the quality at 100k is the sweep-only quality. It is not compared with Graphviz (dot would take far too long there).

## 7. Tests (`cargo test -p family-layout`, 7 tests)
Fast score equals brute force; known score by hand; nuclear family; hard cases (cousin marriage, remarriage, five half-sibling families with zero crossings, adoptive plus birth families, several unconnected trees and a person alone, a partner cycle in bad data); generated trees: permutations, layer rules, determinism; one edit moves little (hint and no hint).

## 8. Things you should know / decide
1. **Trees are laid out one after another**, not layer by layer across trees as the original does. Layer-major creates crossings between unrelated trees; the original's order is still scored as a reference.
2. **Speed.** Big single trees are slow because of network simplex (about 6 s for 8,000 nodes: one pivot recomputes everything) and the refinement. 6,000 people take about 16 s. Options: keep as is and run it in the background and cache the result (the hint makes later edits cheap), or lower the budgets and accept a smaller margin. The caps are constants in `rank.rs` and `order.rs`.
3. `layout_with_hint` still re-ranks; storing the layers in the database would remove that cost later.
4. Samples: the original's `.ged` files are not committed (their licence is unknown); they were read from a scratch clone. `Ragusan.ged` is not UTF-8 and was read as Latin-1 for this test; our importer rejects it by design (ANSEL/other encodings are not supported yet).
5. Windows is still not verified.
