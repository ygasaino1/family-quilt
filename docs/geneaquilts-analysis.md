# GeneaQuilts analysis (Phase 0)

Source: https://github.com/jdfekete/geneaquilt (cloned; 31 commits; version `1.0.13-SNAPSHOT` in `pom.xml`). All paths below are relative to `src/main/java/geneaquilt/` unless they start with `LICENSE`, `pom.xml` or `src/data/`.

**Limits of this analysis.** (1) The original could **not be built or run** in the sandbox: Maven Central returns 403 through the proxy, so no dependencies (Piccolo2D, JUNG, gedcom4j, freehep) can be fetched. Everything below comes from reading code and sample files, not from watching it run. Rows marked *needs run* must be confirmed by building it on your machine. (2) The paper PDF is not linked from aviz.fr and the paper host is not reachable here, so section 5 (code vs paper) is empty.

## 1. License

`LICENSE`: "Simplified BSD License", copyright 2010-2014 Jean-Daniel Fekete, Pierre Dragicevic and INRIA. Every source file carries the same header ("governed by a BSD-style license … found in the LICENSE file"). `pom.xml` also declares BSD.

Surprise: the legal text is actually the **3-clause** BSD text (it contains the "neither the name of INRIA … may be used to endorse" clause), although the file calls itself "Simplified".

Implications: porting the algorithms to Rust is allowed. Required: keep the copyright notice and the conditions and disclaimer in source redistributions (a `NOTICE` file plus a header in each ported file), reproduce them in binary distributions (put them in the app's About/Licenses screen and the README), and do not use the names INRIA or the authors to endorse our product. No copyleft: the license of our own code stays our choice.

Not covered by that license: the sample data files in `src/data/` (the Greek, Bible, Kennedy, Royal92 ... files have their own, unknown, provenance). Do not ship them; use them only as local test inputs, never committed.

## 2. Architecture of the original

| Package | Role |
|---|---|
| `data/` | `Network` (JUNG directed graph), `Vertex` (props map, layer, x), `Indi`, `Fam`, `Edge`, `DateRange` |
| `io/` | `GEDReader` (gedcom4j), `PEDReader`, `TIPReader`, `TESTReader`, `DOTLayersReader`, `LayersReader`/`LayerWriter` (.lyr), `DOTWriter`, `JSONWriter` |
| `algorithms/` | `GenerationRank` (used only as fallback), `LayerRank`, `LayerRank2`, `LayerClusterer`, `VertexOrder` (mostly unused or unfinished), `BFSCycleFinder` |
| `nodes/` | Piccolo2D scene graph: `QuiltManager`, `IndiGeneration`, `FamGeneration`, `PIndi`, `PFam`, `PEdge`, `TimeLine`, `PSemanticText` |
| `selection/` | `Selection`, `SelectionManager`, `DOIManager`, `MouseSelectionController`, `SlidingController` (Bring & Go), `LinkSlidingController` (link sliding), highlight classes |
| root | `GeneaQuilt` (main, menus, search), `BirdsEyeView` (overview), `DetailsTable`, `Printer`, `ConstraintViewport` |

### Data model (`data/`)
A bipartite graph of two vertex kinds: **Indi** and **Fam**. `Fam` = one GEDCOM `FAM` record = one column. Edges (`Edge`, no payload besides from/to):
- child to family: `Edge(indi, famc)` (from `GEDReader.java` lines 112-120)
- family to spouse: `Edge(fam, indi)` for every `FAMS` (lines 123-130)

So husband and wife are both attached with the same kind of edge, and a child is attached with the same kind of edge as a parent. The cell's meaning (parent or child) comes only from its position relative to the diagonal.

### GEDCOM loading (`io/GEDReader.java`)
- Uses gedcom4j. Per individual it copies NAME/GIVN/SURN, aliases, SEX, attributes and **individual** events (tag + `.DATE`, `.ADDR`, `.AGE`, `.CAUSE`, `.DESC`).
- **Only one `FAMC` per person is kept.** Line 63 `indi.setFamc(...)` is called inside a loop, so the **last** `FAMC` wins (the comment says "should take the fist?"). A child with a birth family and an adoptive family loses one of them, silently.
- `PEDI` is never read. grep for `PEDI`, `ADOP`, `FOST` in `src` returns nothing.
- Families: only `HUSB` and `WIFE` are read. `CHIL`, `MARR`, `DIV` and family events are **not read** (the family's children come from the children's `FAMC`).
- Sample files: `greek-myth.ged` has 803 INDI, 65 people with two or more `FAMC` and 11 `PEDI` lines; `Bible Family Tree.ged` has 5 multi-FAMC people and one `PEDI foster`. Both are ideal test inputs for our new feature.

## 3. Layering and ordering

### What is actually used
`GeneaQuilt.load` (lines ~470-490): try `LayersReader` (reads `<file>.lyr`); if absent, `DOTLayersReader` runs `dot -Tplain` as an external process, then `saveLayer()` writes the `.lyr` file (id, x, layer per vertex, via `LayerWriter`). If `dot` is missing the user gets a dialog and layers are wrong.

`io/DOTWriter.java` (bare mode) writes: `digraph genealogy { ranksep=1; node [shape=point,width=0,height=0,label=""]; edge [style=invis]; "<from>"->"<to>"; ... }` with every edge of the network (child to Fam, Fam to spouse). It emits `{rank=same; ...}` groups only for vertices that carry a `GENERATION` property (not set by GED files). Nothing else is requested from Graphviz: **ranks (y) and left-to-right order (x) of all vertices, Indi and Fam together**.

`DOTLayersReader` parses `node <id> <x> <y> ...` lines, stores `x` in the vertex and maps the sorted distinct `y` values to layer numbers 0..k (`binarySearch`). Because the edges point child to Fam to parent and `dot` puts sources on top, in `-Tplain` output the parents have the smallest y, so **layer 0 = oldest ancestors**. Individuals sit on even layers (`layer/2` is the generation) and families on odd layers, or the reverse (`familyFirst`), so a family always sits between its parents' generation and its children's generation.

### Row and column order
`QuiltManager` constructor: `assignLayers()`, then if layers are already set (always the case when dot or .lyr was used) only `updateSortedLayers()` runs, which sorts each layer by `(component, x)`. **So the final order of rows and columns is the x coordinate computed by Graphviz** (mincross + x-positioning).

The built-in fallbacks are not the main path:
- `GenerationRank` (a network-simplex style ranker with cycle reversal) runs only if no layers exist. It is effectively dead code in the shipped flow.
- `QuiltManager.sortLayers()` (barycentre sweeps, max 100 iterations) also runs only without layers. It contains a bug: `if (barycenter != Double.NaN)` is always true.
- `LayerRank`, `LayerRank2`, `LayerClusterer`, `VertexOrder`: unused or unfinished (`LayerClusterer.rank` returns null with a TODO).
- `fixLayers()`: for several connected components, shifts a component by one layer so its last vertex has the same kind (Indi or Fam) as the deepest component's. `fixLayersByDate()` exists but is commented out.

### Pseudocode of the effective pipeline
```
load(file):
  net = read GED/PED/TIP                  # Indi, Fam vertices; edges child->famc, fam->spouse
  if file.lyr exists: (x, layer) = read(file.lyr)
  else: run `dot -Tplain` on the invisible-edge digraph; layer = rank index of y; x = dot x; write file.lyr
  components = weak components of net
  fixLayers()                              # parity shift between components
  gen = layer/2; bucket Indi and Fam per generation
  sort each bucket by (component, x)
  layout: for g in generations (Fam first or Indi first, per layer 0):
      place block, advance x by block width, y by block height   # produces the diagonal
```
Layout constants (`GraphicsConstants`): cell size 14, indi font Helvetica 12 (line spacing 0.85), fam font Helvetica 10.

### What this means for us
- **Exact parity of row/column order is not reachable without reimplementing Graphviz `dot`** (rank by network simplex, then mincross, then priority x-positioning), and even then ties depend on input order. We should not promise identical order.
- Proposal: our own pipeline, in Rust, no external binary: (1) ranking: longest-path layering then network simplex tightening (Gansner et al. 1993) on the bipartite Indi/Fam graph; (2) ordering: barycentre/median sweeps with crossing counting, per component, tie-break by birth date then id; (3) our own diagonal block layout. Use Graphviz 2.43 (present in the sandbox) **only as a dev oracle** for rank assignment. Parity tests then compare: same set of families, same generation for every vertex (up to a constant shift per component), same cell set; not identical order.
- Oracle check done in the sandbox (script `oracle.py`, reproduces the original's graph, last-FAMC-wins): `dot` takes 0.01 s (Simpsons), 0.42 s (greek-myth, 803 people, 57 ranks), 0.09 s (mason), 3.8 s (royal92, 3010 people, 159 ranks). So ranking itself is cheap; the cost we must watch is ordering.

## 4. Visual encoding (exactly as implemented)

| Element | Code | Result |
|---|---|---|
| Row label | `Indi.getLabel` = sex string + NAME | `♂ ` for M, `♀ ` for F, four spaces for anything else (U, X, missing). Helvetica 12, black |
| Column header | `Fam.getLabel` = `" F "` (the male/female and "married" symbols are commented out, "FIXME") | Always the letter F, Helvetica 10, with a white border rectangle |
| Cell shape | `PEdge` constructor | **square if sex is exactly `M`, circle for everything else** (F, U, X, missing). Sex comes from the person in that cell (`Edge.getSex`) |
| Cell fill | `GraphicsConstants.edgeColor()` | filled, `Color(0.25,0.25,0.25)` = dark grey `#404040`, **not pure black** |
| Same mark for parent and child | `PEdge` has no role field | A parent cell and a child cell look identical |
| Grid | `gridColor` 0.75 grey; small grid 0.9 grey | thin lines through rows and columns |
| Zoomed-out rows | `IndiGeneration.fullPaint`, `INDI_GENERATION_COLOR` 0.75 grey | below scale 0.1 a generation is a grey block; only search-highlighted labels still drawn |
| Selection colours | `SELECTION_COLORS`: red, blue, green, olive, purple, teal, grey | one colour per selection (several at once) |
| Path highlight | `PATH_HIGHLIGHT_WIDTH` 9, multicolour stroke | thick coloured path along ancestors/descendants |

Checks you asked for:
- **Outlined (hollow) shapes**: none exist. `PIsoShape` only fills (`g.fill`). So our outlined square/circle does not clash with anything.
- **Grey background behind names**: not used for people. Grey is used only for grid lines, the zoomed-out generation blocks and small text. `PSemanticText` can draw a white translucent background/outline when text outlines are enabled. So the grey name background for deceased people does not clash; the "Text outlines" option (white halo) is a separate thing we keep.
- **Colours already exist in the original** (selection and path colours above). Your rule "no added colours" is about marks; we must decide at Gate A whether selection colours are kept for parity (they are a feature, see section 6).
- Unknown sex: the original draws a circle. Your prompt says "reuse what the original does for unknown sex": that means circle, outlined when non-blood.

## 5. Marriage question (answered)

The original quilt **does not distinguish** married, unmarried, divorced or co-parents:
- `GEDReader` does not read `MARR`, `DIV`, `_MARR` or any family event. `Fam.getMarriage()` reads a property `MARR.DATE` that nothing ever sets, so it is always empty.
- Every `FAM` record becomes one column with the fixed header "F", whether or not the parents are married, divorced, or only one parent exists.
- A `FAM` with only `HUSB` or only `WIFE` gives a one-parent column (one spouse edge). A `FAM` with no children still gets a column: its two spouse cells and no child cells.
- A person with several marriages is simply in several `FAMS` families, so appears in several columns on his or her row (one cell per family). Nothing orders or labels the marriages.
- No tooltip or header text shows dates or place of the marriage. *(needs run: confirm there is no tooltip; grep finds tooltips only in `DetailsTable`.)*

Consequence: the M/U/D/?/P header letters are a **new feature**, not parity.

## 6. Feature inventory (parity checklist)

Status values: `delivered` / `deviation` (needs your written approval) / `open`. "How we deliver it" is the plan. Everything is `open` until built. **No row is dropped.**

| # | GeneaQuilts feature (source) | How we deliver it | Status |
|---|---|---|---|
| 1 | File > Open: `.ged`, `.tip`, `.ped` (`GeneaQuilt.showFileChooser`) | GEDCOM import (required). `.ped` and `.tip`: importers in Phase 2 (small formats, ped has 4 sample files) | open (ped/tip not in your prompt: please confirm) |
| 2 | Layer cache `.lyr` (`LayerWriter`) | Not needed (our layout is computed; cache in SQLite if slow) | open, proposed deviation |
| 3 | Graphviz `dot` for layers | Own ranking, no external binary | by design (your section 3) |
| 4 | File > Export to JSON (`JSONWriter`) | JSON export of people and links | open |
| 5 | File > Export to DOT, Export selection to DOT (`DOTWriter`) | DOT export (whole tree, selection) | open |
| 6 | File > Export Graphics to ... (`Printer`, freehep PDF) | PDF/PNG/SVG export of the current view | open |
| 7 | File > Stats (generations, families, individuals, edges) | Stats dialog | open |
| 8 | File > Quit with confirmation | Window close + confirm if unsaved work | open |
| 9 | View > View All | Fit button | open |
| 10 | View > Label by Name / Surname / Given name | Label selector | open |
| 11 | View > Text outlines (white halo for labels over selections) | Same option | open |
| 12 | View > Keep DOT file, Frame Rate, Thread Bugs (debug) | Frame-rate overlay only; the other two are Java/Graphviz-specific | open, proposed deviation (2 of 3 meaningless) |
| 13 | Mouse wheel zoom (`MouseWheelZoomController`, 10% per notch) | Wheel zoom + -/+ buttons | open |
| 14 | Drag to pan (`DisabablePanEventHandler`) | Same | open |
| 15 | Click selects a row/cell/column; shift-click adds; drag moves selection; click on empty space clears (`MouseSelectionController`) | Same | open |
| 16 | Multiple simultaneous selections, 7 colours (`SELECTION_COLORS`) | Same colours | open (colour decision) |
| 17 | Click on a selection cycles highlight mode: all, none, ancestors only, descendants only (`Selection.toggleHighlightMode`) | Same, plus your blood-lineage / family-lineage toggle | open |
| 18 | Bring & Go + link sliding navigation (`SlidingController`, `LinkSlidingController`, `PSemanticPath`) | Re-implement; spec from code, tested on screenshots | open (largest interaction risk) |
| 19 | Search box with field selector, "Literal", "NoCase" flags (`GeneaQuilt` search UI) | Search box with field, literal, case options | open |
| 20 | Filter toggle: fade/hide items by degree of interest, 500 ms animation (`DOIManager`, `PFilterAnimation`) | Same, Dijkstra-style DOI over the same graph | open |
| 21 | Overview "bird's eye" window: drag viewport, shift-drag pans, left/right arrows walk along the hull path (`BirdsEyeView`, `Hull`) | Minimap with the same three behaviours | open |
| 22 | Timeline strip from date ranges, click selects, double click picks (`TimeLine`, `DateRange`) | Timeline strip | open |
| 23 | Details table of the selected items with tooltips (`DetailsTable`) | Sidebar (richer) | open |
| 24 | Zoom-dependent rendering: grey generation blocks below scale 0.1, labels greeked when small (`IndiGeneration`, `PSemanticText`) | Same idea in canvas | open |
| 25 | Several components stacked, layer parity fix (`fixLayers`) | Same effect | open |
| 26 | Cycle handling (`BFSCycleFinder`) | We forbid cycles at write time; import must report cycles | open |
| 27 | Print (commented out in menu) | Covered by row 6 | n/a |
| 28 | Estimate dates (commented out) | none | n/a |

Confirm by running: keyboard shortcuts besides arrows in the overview, exact Bring & Go gesture, whether tooltips exist on cells.

## 7. Non-biological relationships in the original
- None supported: no `PEDI` handling, one family per child (last `FAMC` wins), no distinction between birth and adoptive parents, and a person who is a child in two families keeps only one child cell.
- In the sample data this changes the picture: in `greek-myth.ged` 65 people are children in several families, so the original drops 65 child cells (it draws one parent family per person).

## 8. Where prompt and code disagree
1. Prompt: "filled black" marks. Code: dark grey `#404040`.
2. Prompt: "verify filled square male / circle female". Code: square only for `M`; **every other sex, including unknown, is a circle**.
3. Prompt: "its license ... unclear". Code: BSD, in fact 3-clause.
4. Prompt: header letter replaces "F". Code: F is a fixed `" F "` label (OK, easy).
5. Prompt/earlier note listed a separate "S" for separated; your section 7 says none (ended = D). I follow the prompt (no S).
6. Prompt: "child with more than one family is treated ...". Code: treated as a child of only the last one.
