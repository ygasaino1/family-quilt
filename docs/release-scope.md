# Release scope proposal (for your row-by-row approval)

Source: the Phase 0 feature inventory (`docs/geneaquilts-analysis.md`, section 6). **No row is dropped**: "after release" rows stay on the list and the project is not complete until every row is delivered. Gate D (parity) moves to after the first release.

Effort: **S** = about a day, **M** = 2-4 days, **L** = a week or more (my estimate, including tests). "Without it": how usable the quilt is if this row is missing.

Already delivered in earlier phases: GEDCOM import/export (row 1, GEDCOM part), cycle handling (row 26), stacked components (row 25, part of the layout engine), layer-by-layer mode (original's generation bands).

| # | Feature | Proposed | Effort | Without it |
|---|---|---|---|---|
| 1 | Open `.ged` (done); `.ped` and `.tip` importers | v1: `.ged`. After release: `.ped`, `.tip` | S each | Fully usable: GEDCOM is what family software exports |
| 2 | `.lyr` layer cache | Dropped at Gate A (our layout is cached in the file) | - | n/a |
| 3 | Graphviz `dot` for layers | By design: own ranking (done) | - | n/a |
| 4 | Export JSON | v1 | S | Usable, but no plain-data way out except GEDCOM and backup |
| 5 | Export DOT (whole tree, selection) | After release | S | Fully usable: developer feature |
| 6 | Export graphics (PNG/SVG/PDF) | v1: PNG and SVG of the current view. After release: PDF | M | Usable on screen; family cannot print or send a picture of the quilt |
| 7 | Stats dialog | After release | S | Fully usable |
| 8 | Quit with confirmation | v1 (window close; edits are saved at once, so the confirm only guards open dialogs) | S | Usable |
| 9 | View All (fit) | v1 | S | Painful on big trees: must zoom by hand |
| 10 | Label by Name / Surname / Given name | v1 | S | Usable; wide labels on big trees |
| 11 | Text outlines (halo over selections) | v1 | S | Labels harder to read over selections |
| 12 | Debug items (Keep DOT file, Thread Bugs); Frame Rate overlay | Dropped at Gate A for the two Java/Graphviz items; frame-rate overlay after release | S | n/a |
| 13 | Wheel zoom + -/+ buttons | v1 | S | Unusable without it |
| 14 | Drag to pan | v1 | S | Unusable without it |
| 15 | Select row/cell/column; shift-click adds; clear on empty click | v1 | M | Unusable without it |
| 16 | Several selections, 7 colours | v1 | S | Usable with one selection; loses the main way to compare branches |
| 17 | Highlight-mode cycling (all, none, ancestors, descendants) + blood/family toggle | v1 | M | Usable; relationships are much harder to see |
| 18 | Bring & Go + link sliding navigation | After release | L | Usable: search, minimap and click-select cover navigation. Biggest interaction risk, so best built on a stable v1 |
| 19 | Search (field selector, literal, case) | v1: name search, literal and case options. After release: the other fields | M | Unusable on big trees |
| 20 | Filter: fade/hide by degree of interest, 500 ms animation | After release | M-L | Usable on family-sized trees; on very large ones it is the main decluttering tool |
| 21 | Overview window | v1: minimap with viewport drag and click-to-jump. After release: shift-drag pan and arrow keys along the hull | M | Usable; harder to orient on big trees |
| 22 | Timeline strip | After release | M | Fully usable |
| 23 | Details table / sidebar | v1: profile sidebar (spec section 9) | M | Unusable (it is where you read and edit a person) |
| 24 | Zoom-dependent rendering (grey blocks, greeked labels) | v1 (needed for the 60 fps target) | M | Too slow on big files |
| 25 | Stacked components | Delivered (layout engine) | - | - |
| 26 | Cycle handling | Delivered (forbidden at write time, reported on import) | - | - |
| 27 | Print | Covered by row 6 | - | n/a |
| 28 | Estimate dates | n/a in the original (commented out) | - | n/a |

## Not inventory rows, but in v1 anyway (other parts of the spec and your scope note)

Window and first-run wizard, password unlock, review screen with before/after and conflicts, history and undo in the sidebar, privacy mode, backups (rolling + one-click), settings including "AI assistant (advanced)", the translation mechanism (`t()`, `en.json`, no hard-coded strings), text-size setting, keyboard navigation, 44 px targets, light/dark.

## Where I split a row (needs your yes)

Rows 1, 6, 12, 19 and 21 each have a v1 part and an after-release part, so no part is dropped, only scheduled.
