# Decisions log

## Gate A (answered 2026-11-17)
1. Keep the 7 selection colours (parity). Not a deviation.
2. `.ped` and `.tip` importers: included at my discretion (low cost, after GEDCOM). Status: delivered later in Phase 2, may slip without blocking.
3. Row order: owner's requirement is that quality must not decrease. Working rule adopted: structure parity (families, generations, cell sets) plus a quality gate: on every sample file, our edge-crossing count and total row-line length must be no worse than the Graphviz-derived order. If we cannot meet that, we say so at Gate D instead of shipping a worse layout.
4. Debug items (Keep DOT file, Thread Bugs) and the `.lyr` cache dropped; Frame Rate overlay kept. Approved by owner ("I trust your judgment").
5. License for new code: MIT. Ported GeneaQuilts parts keep the BSD notice (NOTICE file, headers).

## Gate C (answered 2026-11-17)
- Canvas 2D accepted, on the condition that selection, lineage-path highlighting and panning do not lag; otherwise WebGL.
- Evidence (headless Chromium, software raster, 10k people): select-to-paint 18-30 ms; 60 fps panning with realistic lineages; stays near 56-60 fps up to ~25k highlighted segments, 43-52 fps at 50k, 24-38 fps at 100k.
- Rule: build the highlight layer behind a small renderer interface. If a Windows run with >= 25k highlighted segments drops below ~55 fps, move only that layer to WebGL.
- Still unmeasured: real window (wry/WebView2), binary size, startup time. Measure on the first Windows build.

## Next step
Phase 1 needs crates.io. Network for package installs was blocked in the Phase 0 chat. Owner will enable it and start a new chat; start Phase 1 there.

## Phase 1 (2026-11-18) — decisions taken, please confirm or overrule
1. **Data key + recovery key.** SQLCipher uses a random 256-bit data key. `<tree>.keys` holds it wrapped twice (password via Argon2id, recovery key) with XChaCha20-Poly1305. Changing the password re-wraps the key (atomic swap) instead of `PRAGMA rekey`; the database file is not rewritten. Consequence: an attacker who kept an *old* key file and knows the *old* password can still open the data. Optional later feature: rotate the data key (re-encrypt) when a password is believed leaked.
2. **Key file is a sidecar**, not inside the database. Backups copy it next to the backup. Whether a tree is protected is decided by the database header, never by the sidecar's presence.
3. **SQLCipher build:** `rusqlite` feature `bundled-sqlcipher-vendored-openssl` (OpenSSL compiled from source; needs Perl at build time, no system OpenSSL). Verified on Linux only; Windows MSVC still to be verified on your machine (see docs/phase1-report.md).
4. **Schema deviations from section 5 (all small):** `person.note` added (section 19 edits it); `updated_at` on `relationship` (section 19 staleness); `deleted_at` on `person_name`, `place`, `event`, `source`, `citation` (soft deletes everywhere); `search_key` on `person_name` and `name_key` on `place` (Persian/Arabic-tolerant search); `relationship.kind` is NULL for partnerships and `subtype` NULL for parent-child (enforced by CHECK) instead of `kind NOT NULL DEFAULT 'birth'` on every row; `UNIQUE(type, person_a, person_b)` is a partial unique index over live rows so a deleted link can be entered again; partnerships are stored with the smaller id first (CHECK). The prototype's per-person `privacy` column was not carried over (privacy mode is a global setting).
5. `change` and `revision` tables exist in migration 1 (logic for `change` comes in Phase 4). `vault_id` is on every table except `meta`.
6. Death date and `is_living = true` contradict each other: the store rejects it (importer must decide).
7. Sorting dates: earliest possible day of the stated precision; the qualifier does not move the position.
8. Nobody can create a person without a name; the last name of a person cannot be removed.

## Phase 2
- Merge rule: union of two groups is exactly 2 parents and they have any recorded partnership (awaiting Gate B).
- Partner who is not a linked parent is not pulled into a column.
- Import is planned first (`ImportPlan`) and applied in one transaction (`Store::atomic`); invalid links are skipped with line-numbered warnings.
- Cycle check query pinned to `idx_rel_b` with `INDEXED BY` because the planner otherwise scanned all parent links.
- Own round-trip data goes in `NOTE family-quilt:*` and `_FQ*` tags, which other programs ignore.

## Phase 3
- Gate B accepted as proposed.
- `family-relate` works on the `Graph` snapshot only; no SQL. No new external dependency.
- Half-sibling/half-cousin = exactly one recorded shared common ancestor.
- Non-birth steps cost 1 + 0.001 so a birth route wins ties.

- Half-sibling: confirmed by the user, keep as is (exactly one recorded shared parent reads half-sibling).

## Layout engine (before Phase 4)
- Score = quilt line crossings + row-line length (+ column-line length reported), defined on the final row/column order only.
- Trees are placed one after another (largest first, people alone last), not layer-major across trees.
- Ranking: network simplex on person -> column -> child (trees up to 12,000 nodes); simple compaction above that.
- Ordering: depth-first start, mean/median sweeps (4 variants), neighbour-swap refinement on the real score within a deterministic work budget.
- `layout_with_hint` keeps the previous order for stability.

- Layout modes (user decision): both tree-by-tree (default) and layer-by-layer are available. Layer-by-layer defaults to `Align::Top` (the original, no dates needed). `Align::BirthDate` stays only as an optional extra.

## Gate E closed (user)
- Option A for big trees: show a quick layout first (stays still, user can scroll/search/select), compute the careful layout in the background, cache the result with the file, keep the order after edits (hint).
- Progress strip with real stage + score ("Ranking generations... ordering... polishing (crossings a -> b)") and a "keep current layout" button; one smooth ~0.5 s re-arrangement at the end, selection stays on the same people; instant if the system asks for reduced motion.
- Optional setting "show live progress" (off by default): a few intermediate states (after ranking, ordering, polishing).
- Engine to add later (app side): progress callback with stage, fraction, current score, and a snapshot at those points.
- Release scope proposal (v1 / after release per inventory row) is due before Phase 5; Gate D moves after the first release.

## Phase 4 (HTTP, both doors, proposals, MCP)

**Crates.** `family-review` (no HTTP: submit/validate, review view, warnings, approve/reject, query side) and `family-server` (axum; `/api`, `/agent/v1`, `/agent/mcp`, the `family-app` binary with `--mcp-stdio`). All SQL stays in `family-store` (`review.rs`: proposals, changes, undo).

**Doors.** Every request needs a localhost `Host` (and a localhost `Origin` if one is sent). `/api/*` additionally needs `Sec-Fetch-Site: same-origin|none` (missing = refused). `/agent/*` needs the bearer token (constant-time compare). The agent token does not open `/api`. Known limit: a native program on this PC can forge `Sec-Fetch-Site`; the header protects against websites, not against local malware, as the spec intends.

**Agent powers** are defined once in `ops.rs`; HTTP and MCP both call it: guide, search, person, relationship, graph, create proposal, list own proposals. No route or tool exists for edit, delete, approve, reject or undo (tested: 404/405 on the HTTP side, tool list asserted on the MCP side).

**Proposals.** Strict validation with field-level errors (`path`, `code`, `message`); nothing is stored unless everything is valid. New people and links are stored as `pending` rows; edits are stored only as `change` rows (`{field: {from, to}}` + `base_updated_at`); verified rows are never touched until approval. Decisions I made where the spec was silent:
- Links and edits may target people/links that are still pending in another proposal (spec section 6 already said approval is then blocked; my first version was stricter than the spec and is corrected). The review shows `depends_on`; approving is blocked until that proposal is approved; rejecting it removes the dangling links, marks the dependent edits `conflicting`, and reports which proposals were affected.
- `from` is checked at submit time too (`stale_from`), so an agent that read old data learns at once.
- An agent may have at most 50 open proposals; limits per proposal: 200 people, 400 links, 200 changes; body limit 1 MB.
- Editable fields are whitelisted per target type (person, person_name, relationship). Merging people, `id`, `status`, `vault_id`, `created_by`, `proposal_id`, `merged_into`, timestamps are refused.
- Approval is one transaction. Without explicit decisions, all non-conflicting fields of all changes are applied; a conflicting field is skipped and reported, and is applied only if the human lists it under `overwrite`. Rejection soft-deletes the proposal's pending records.
- Warnings are computed on the resulting state, only for what the proposal touches: parent too young (<12) or too old (mother >55, father >80), parent died before the birth (father: 280 days of slack), death before birth, possible duplicate (same name), and the link-kind impact note ("removes N people from X's blood lineage and changes up to M stored relationship answers"). Cycles are refused by the store when the link is written.
- `?pending=1` applies open proposals' changes on top of the graph (kind, subtype, end date, removals, dates, sex) and the result says `includes_pending`.
- Undo restores the old row as a **new** revision; refused if the record changed again since. (Bug found by test and fixed: restoring column by column broke the date/qualifier CHECK; it is now one statement.)

**MCP check (spec section 18), done on 2026-11-19.**
- The current MCP spec has two eras of Streamable HTTP. 2025-03-26 to 2025-11-25: `initialize` handshake, optional `Mcp-Session-Id`, optional GET stream. The newest revision (2026-07-28) removes sessions and the GET stream and puts client info and protocol version into every request.
- We implement the 2025 era (JSON responses, session id minted on `initialize`, GET/DELETE answer 405). A client that sends `MCP-Protocol-Version: 2026-07-28` gets the spec's 400 "unsupported version" with our supported list so it can fall back. Adding the newest revision is a contained change once clients use it.
- Origin is validated (403 for a non-localhost Origin) as the spec requires for DNS-rebinding protection.
- Claude Code supports stdio, HTTP (`claude mcp add --transport http <name> <url> --header "Authorization: Bearer ..."`) and SSE (deprecated). Checked against Anthropic's Claude Code docs.
- Claude Desktop: I could not fetch Anthropic's own page (blocked here). From what I know and third-party pages, it starts local servers from `claude_desktop_config.json` over **stdio**, and reaches remote servers through its Connectors UI. That is why the bridge exists. **Unverified in this session; please confirm on your machine.**
- The bridge answers `initialize`, `tools/list`, `ping` and the guide locally, so clients show it as healthy, and only `tools/call` needs the running app. With the app closed a call returns "Open the Family Tree app first".

**Open for later phases.** Window, wizard and password unlock in the app (the headless binary refuses a protected tree for now); settings screen is only the JSON endpoint; push instead of the 5 s poll is not needed (the UI polls `/api/status`); relationship/graph reload the graph per request (fine for family-sized trees, to be cached for 100k).

**Person removal by proposal (your request, after Phase 4).** Spec section 19 said deleting people stays human "for now"; you asked for it, so `changes` now accept `op: remove` on a person (merge stays human-only; `delete`/`merge` op names are refused with a hint). Safeguards: it is a soft delete and undoable from History; the review shows an impact note ("removes N links, and M people would be left without a recorded parent (names)"); it is **never applied by default**: approving without an explicit tick leaves the person alone and the report lists `removals_not_confirmed`; a stale person needs an explicit overwrite; pending people cannot be removed this way (reject their proposal instead); the agent guide and tool description say to propose it only when the user explicitly asks. Tests: 14 in `family-review`.

## Transport decision (user, 2026-11-21): hybrid + headless

**Chosen: B + D.**
- **B (hybrid).** In the real window the UI talks to the backend in-process through wry's custom protocol: the same axum `Router` answers the window's requests, no UI port exists, and no per-launch UI token is needed (only our own window can reach the UI routes). The agent door stays on `127.0.0.1` with its bearer token. Playwright and headless runs use the same router over a TCP port, with today's guards (`Host`, `Sec-Fetch-Site`).
- **Fallback.** If WebView2 misbehaves with custom schemes (unverified until the first window is built on Windows, early in Phase 6), the UI goes back to localhost HTTP plus a per-launch token. The front end does not change either way.
- **D (headless).** `family-app --data DIR` starts only the server. The bind address is configurable; localhost is the default and any other value prints a loud warning. Remote use is, for now, an SSH tunnel or a private network (Tailscale, WireGuard); no login system is added.

**Remote/hosted access is not planned.** It is a possible future feature only: not on the phase list, nothing decided. It would need login and sessions, HTTPS, per-user permissions, rate limiting, server backups and an encryption decision. The design must not block it (no assumption that the window and server share a machine), nothing more.

## Phase 5 (quilt renderer, sidebar, review UI, translation)

**Where things are.** `web/` (plain HTML/CSS/JS, embedded in the exe; the server also answers `/locales/*.json`). Modules: `view.js` (Canvas 2D renderer + pan/zoom/selection), `quilt.js` (data model, ancestors/descendants, labels), `minimap.js`, `sidebar.js`, `review.js`, `settings.js`, `export.js`, `i18n.js` (`t()`, dates, `qps` pseudo-language), `prefs.js`, `dom.js`. Names and other data are always inserted with `textContent`; the page has a strict Content-Security-Policy (`default-src 'self'`).

**Server additions** (all under `/api`, same guards): `GET /quilt` (+ progress, cancel), `GET/PUT /settings` (tree title, stored in the file), `GET /export/json`, `GET /export/gedcom`, `POST /import/gedcom`, `PATCH /names/{id}`; `--port` for the headless binary.

**Rendering (rebuilt on 2026-11-21 after studying the original's `QuiltManager`, `IndiGeneration`, `FamGeneration`, `PEdge`).** The earlier version was a flat matrix with boxed cells; the original is not. `web/js/geometry.js` holds the rules:
- **Staircase.** Rows and columns are grouped in alternating blocks along a diagonal: people of generation g (rows), the families of g (columns), people of g+1, and so on; each block starts right of and below the previous one. In tree-by-tree mode each tree has its own staircase, in layer-by-layer mode one staircase serves all trees. (Same as the original's `layoutChildren`.)
- **Names sit between column groups**, left-aligned in a slot as wide as the longest name of that generation. A person's mark as a child is left of the name, their marks as a parent are right of it. The test `geometry follows the original` checks this for every cell.
- **The header band sits between the parent rows (above) and the child rows (below)** and is only as wide as its column group. One header cell per column, with the state letter (M, U, D, ?, P) from the translation table.
- **No cells, only rows and columns.** The grid is one line between consecutive rows and one between consecutive columns, each as long as the union of its two neighbours (the original's `createIndiLines` / `createFamLines`). A row strip spans the name and all of the person's marks, a column strip spans the header and all marks; what looks like a cell is a crossing. Strips are filled with the "cell" colour on the grey stage; selection bands cover exactly the strip.
- Marks: square = man, circle = woman, diamond = other/unknown (the original draws a circle for unknown; kept as before), filled = parent or birth child, hollow = non-birth child; pending person = amber frame and hatched marker, pending link = amber circle with a slash. Names are drawn at a fixed size in world units, so they scale with the zoom (greeked below 8.5 px). A grey name background means a recorded death.
- Culling: row y and column x are strictly ascending, so the visible range is found by bisection; the glide interpolates the four position arrays (row y and name x, column x and header y). 59-60 frames/s on 10,756 people / 5,151 columns, draw time under 0.3 ms.
- Not copied from the original: its zoomed-out grey generation blocks (lines thin out instead), its fixed header "F" (ours is the partnership state).

**Chrome (matches the UI2 mock-up).** Left icon rail (tree, people, review with badge, agent, settings), top bar (back/forward through selections, tree title, search with prev/next, export, privacy eye, bell), floating zoom controls with a View popover (label mode, layout mode, highlight, blood toggle, search options, outlines), minimap and legend. Review is a docked panel beside the quilt (agent and time, your words, what was understood, suggested changes with Accept ticks, warnings, approve/reject). Per-item exclude icons are done (see "Leaving items out of an approval"); editing an item in place is not.

**Selection.** Click a cell or name = that person's row; click a header = that column's family; shift-click adds another selection in the next free colour (7 colours); a click on empty space clears. Selection never moves the view (tested). Highlight modes cycle (ancestors and descendants / only the selection / ancestors / descendants) with a blood-only / whole-family toggle (blood = birth links only). During the end-of-layout glide, clicks pick what is drawn, not where it is going.

**Option A in the UI.** The quick layout is shown at once; a strip shows the real stage and score ("Polishing (crossings a → b)") with "Keep current layout"; when the careful layout is ready, one glide of 0.5 s moves rows and columns (instant with reduced motion); selection and view stay put (tested). "Show live progress" (Settings, off by default) shows the intermediate arrangements.

**Choices I made where the spec was silent.**
- Given-name and surname labels are split from the display name (surname = last word). The store has structured names, but the quilt payload carries only the display name; this will be wrong for some names (particles, Persian order). Proper fix: send `given`/`family` in the payload.
- The relationship sentence ("X is the ... of Y") comes from the server in English; it is not yet in the translation table.
- History in the sidebar is per person; the global history list is only an API for now.
- Sex is a letter in the quilt payload (M/F/X/U) and a word in the people API; `sexWord()` / `sexLetter()` in `dom.js` are the only conversion.
- The page keeps theme, text size, language, privacy mode and label/highlight choices in the browser (per viewer); only the tree title lives in the file.
- Privacy mode greeks the names on the canvas and blurs the sidebar, tooltip and lists until hovered or focused.

**Checks run.** `web/tests/e2e.mjs` (Playwright): symbols by pixel, hollow cell, header letter, select without view jump, shift-select with its own colour and "how related", search, agent proposal appearing (badge, toast, pending look), review (your words, before/after, picture, approve; reject changes nothing), keyboard, 44 px targets, highlight modes and blood toggle, label/layout/privacy, big file with progress strip, one glide and keep-current, dark theme, settings/export/agent details, no missing translation keys, pseudo-language. `web/tests/keys.mjs` and `crates/server/tests/web.rs` (every file in `web/` is embedded; CSP; forged Host refused).

**Performance (10,115 people, 4,800 columns).** In headless Chromium (software rendering, so conservative for a real GPU): 59-60 frames/s while panning at four zoom levels, 0.1-0.5 ms of drawing per frame, zoom steps under 2 ms. Quick layout 0.6 s (debug build); the GEDCOM import took 9.9 s in a debug build. Not yet measured on Windows/WebView2.

**Not done in this phase (Phase 6 or later).** The window itself (wry), quit-with-confirm, first-run wizard, password unlock screen, automatic backups, packaging; the custom-protocol transport (hybrid decision); selecting people with the keyboard alone (arrows pan, `+`/`-`/`0` zoom, search + Enter selects, but there is no row-by-row keyboard focus yet); a text-size test beyond the setting itself.

## Layout engines: the Graphviz layout, built in (2026-11-22)

The original GeneaQuilts gets its layers and its order inside each layer from Graphviz `dot`. Ours looked less neat, so
we ported the two stages that matter, from the Graphviz 16.1.2 sources, into Rust (no program to install):

* `crates/layout/src/dotrank.rs`: class1, decompose, acyclic, network simplex (feasible tree, cut values, pivots) and
  the top-bottom balance; this fixes each node's layer.
* `crates/layout/src/dotorder.rs`: class2 (virtual node chains), build_ranks, weighted medians, reorder, transpose and
  the merge of connected components; this fixes the order inside each layer.

Only what the quilt needs is ported (plain nodes, no clusters, ports, labels or flat edges); splines and the x
coordinates are not needed because the original only reads the layer and the order.

* **Verified against `dot` 16.1.2** (built from the same sources): identical layers and identical order in every layer on the
  Greek gods file, 800 random graphs (cycles, duplicate edges, several components) and a generated 5,600-node
  genealogy. Developer tool: `cargo run -p family-layout --example dotrank_cmp -- FILE.dot`.
* One detail that matters: cgraph keeps a node's out-edges ordered by the *head's* creation number first, then by the
  edge's. Getting this wrong changed ties in a third of the random graphs.
* Edge order: the original's own order depends on a Java hash map, so there is no "the original order" to copy. We
  write the edges in the original's shape (per person: the edge to its parent family, then the edges from the
  families it is a parent of), people in our order.
* A family without parents is handled as in the original (`familyFirst`): the diagonal starts with a block of columns.
* Engines in the View menu: *Built in* (annealing), *Graphviz layout (built in)* (`engine=port`),
  and *Graphviz dot program* (`engine=dot`, optional, only when a `dot` is found). Both Graphviz engines fall back to
  the built-in one when they cannot be used. The Graphviz layout (`engine=port`) became the default after the user reviewed the comparison (2026-11-22).
* Licence: Graphviz is distributed under the Eclipse Public License 1.0. The port is derived work, so it is credited in
  `NOTICE` and the module headers; the EPL terms have to be checked before release.

## Watching Graphviz for changes to the files we ported (2026-11-22)

The Rust ports in `crates/layout/src/dotrank.rs` and `dotorder.rs` are based on Graphviz 16.1.2 (tag `16.1.2`,
commit `445239ae6d37897a48adf308c8172b90162ff0bb`). A daily GitHub Actions job (`.github/workflows/upstream-watch.yml`)
tells us when upstream changes the C files we actually ported, so the port cannot silently fall behind.

- `UPSTREAM`: repository, branch, and the baseline commit our port is based on.
- `UPSTREAM_PATHS`: only the C files we read and ported, each with its Rust file, what was taken, and the evidence
  (H = header/comment in our code names it, C = commit/doc says so, O = we compared output with dot). Files we only read
  for context, or may port later, are deliberately not listed.
- `tools/upstream-check.sh` does the check (partial clone, no file contents copied into this repo) and writes the report;
  `tools/test-upstream-check.sh` tests it on local fixtures (no change, unrelated change, relevant change, missing
  baseline, rewritten history, fetch failure, empty path list).
- One open issue labelled `upstream-review` is kept per set of changes (fingerprint in a hidden marker): no duplicates,
  updated when new relevant commits appear. Unrelated upstream changes never open or update an issue.
- A fetch failure, a missing baseline or a baseline no longer on the branch **fails the run** instead of passing quietly.
- Nothing is ever advanced automatically. To catch up: review the issue, build the newer `dot` and run
  `tools/graphviz-compare/compare.py` (see its README), port what differs, then edit `COMMIT`/`TAG` in `UPSTREAM`.
  Closing the issue without moving the baseline makes the next run open a new one.
- Limits: path-based monitoring flags harmless refactors too (a human decides); the workflow was tested locally only,
  not against the real GitLab (the sandbox cannot reach it). The tag may be an annotated tag object, so the check
  resolves it with `COMMIT^{commit}`.

### Size cutoff for the Graphviz layout (2026-11-23)

Measured in a release build: the port's ordering stage grows much faster than the file (about 5 s at 10k people,
18 s at 18k, 28 s at 21k, minutes at 36k); real `dot` is 3-4x slower still (23 s at 10k, 67 s at 18k). The algorithm
is the cause, not the port. Above 15,000 people (`FAMILY_PORT_AUTO_MAX` overrides, used by the tests) the server does
not start it on its own: the quick layout, then the built-in careful layout, are made as usual and the View menu says
so and offers "Run the Graphviz layout anyway" (`force=1`, kept until the engine is changed). The external `dot`
engine is an explicit choice and is not limited.

### Zoomed-out drawing: a hybrid (2026-11-23)

By the on-screen width of one column (`LOD_BLOCKS` 0.6 px, `LOD_GRID` 3 px in `web/js/view.js`): below 0.6 px each
generation is one solid grey block (as in the original); up to 3 px the grid fades out and the marks stay (never
thinner than a screen pixel); above that everything is drawn. This replaces "lines thin out" in the earlier note.
The thresholds are first guesses, to be tuned by use after v1.

### Leaving items out of an approval (2026-11-23)

The review panel has a ✕ on every new person and link of a proposal. `POST /proposals/{id}/approve` takes
`exclude_people` and `exclude_relationships` (ids must belong to the proposal, otherwise a field error and nothing
changes). A left-out person takes their links along; this proposal's edits to left-out items are marked rejected; links
other pending proposals made to a left-out person are removed and those proposals are reported (`affected_proposals`),
the same way rejecting does. Left-out records are soft-deleted with revisions tagged with the proposal, so the
approval stays one undoable batch. Editing an item in place (changing what the agent proposed) is still not done.

## Phase 6, slice 1-2: first run, lock, passwords (2026-11-23)

- `family-app` starts with no tree open. An unprotected tree opens at once; a missing one shows the first-run wizard; a
  protected one shows the unlock screen. While nothing is open the shared store is an **empty in-memory placeholder**
  and every data route on both doors answers `423 locked`, so agents cannot get around the lock.
- `/api/tree` (state), `/create`, `/unlock` (password or recovery key), `/lock`, `/reset` (recovery key + new password),
  `/protect`, `/unprotect`, `/password`, `/auto-lock`. Passwords are checked before anything is closed, so a wrong
  "current password" changes nothing. Locking drops the store (the key is zeroed) and the cached quilt pictures.
- Auto-lock is a setting in the file (`setting:auto_lock_minutes`); only requests from the window count as activity
  (the 5 s status poll and the layout progress poll do not).
- The wizard asks for a name, an optional password (at least 8 characters) and an optional GEDCOM file, and shows the
  recovery key once with copy, save-as-file and print. **Not yet:** choosing the data folder (it shows the folder in
  use; the choice needs the native window's folder picker), and the portable marker is only read at start-up.
- `--fast-kdf-for-tests` (cheap key derivation, no pause after a wrong password) exists for the end-to-end tests only.

## Phase 6, slice 3: window, launch token, Windows CI (2026-11-23)

- Modes of `family-app`: default (native window, when built with cargo feature `window`; otherwise falls back to the
  browser), `--browser`, `--headless` (server only, used by tests and smoke runs), `--mcp-stdio`.
- The window (wry + tao + rfd) is behind the `window` feature so the normal build and tests need no WebKitGTK.
- Each launch makes a random token. The window gets it as a one-time `?t=` link that sets an HttpOnly, SameSite=Strict
  cookie (`fq_ui`); `/api` on the UI door needs that cookie. Other programs on the PC cannot use the UI door by guessing
  the port. The agent door keeps its own token.
- A second copy is detected through `agent-endpoint.json` and told to use the running one.
- `.github/workflows/windows.yml` (windows-latest): tests, release build with `--features window`, import check (system
  DLLs only), `tools/app-smoke.py` (headless, Persian-named folder, password, lock), a window-start step, exe artifact.
  First run was green on every step.
- **Verified only on CI:** the window itself has not been seen by anyone. The CI step only proves the process starts
  and the service answers. Next check: run the CI-built `family-app.exe` on the user's Windows PC.

## Portable by default; choosing the tree's folder (2026-11-23)

- **No installer for v1.** The release is a single `family-app.exe` (in a zip with a short README). Reasons: nothing to
  install besides the exe, the tree is a file people copy and back up, and an installer would not remove the "unknown
  publisher" warning (only code signing does). Revisit only if people lack the WebView2 runtime or ask for shortcuts and
  `.ged` association.
- WebView2 missing: the window code checks the runtime first and shows a plain message with the download link, and
  `--browser` is offered as the way out.
- Data folder: `--data`, `FAMILY_DATA_DIR`, a file named `portable` next to the exe (data goes in `data/` beside it),
  otherwise `%APPDATA%\FamilyTree`.
- **First-run folder choice.** The wizard shows where the tree will be kept and has "Change…": the native folder dialog
  in window builds, a typed full path otherwise. `POST /api/tree/location` (first run only, 409 once a tree exists
  there) creates and test-writes the folder, remembers it in `location.txt` in the program's data folder, and uses it
  from then on. A folder that already holds a tree is used (and opened if unprotected). A remembered folder that has
  vanished falls back to the default place. A tree is never moved behind the person's back.
- The agent token and endpoint files stay in the program's data folder, not beside the tree.
- The folder dialog (`rfd`) runs on a worker thread; fine on Windows, would need the main thread on macOS (not a v1 target).

## Phase 6, slice 4: lock button, close guard, icon, portable package (2026-11-24)

- **Rail lock button** (padlock above Settings), shown only while the tree has a password; Settings still has "Lock now".
  Setting or removing a password shows or hides it at once.
- **Closing the window.** The native close request is passed to the page (`window.fqCloseRequested`); the page asks
  "Close anyway?" only if a dialog is open (edits are saved the moment they are made, so only a half-filled form can be
  lost), then tells the window to quit over the web view's message channel. If the page cannot answer, the window closes.
  Idle auto-lock and the lock on exit are unchanged.
- **Icon.** A small quilt (generations as rows of patches), drawn by `tools/make-icon.py` into `crates/server/assets`
  (`icon.ico` for the exe, `icon-64.rgba` for the window). Windows builds embed it through `build.rs` (winresource); a
  missing resource compiler only warns.
- **Package.** CI uploads `family-tree-windows-portable`: `family-tree/family-app.exe`, an empty `portable` marker (data
  goes in `data/` beside the exe) and `README.txt` from `dist/`. Downloads from GitHub arrive zipped.
- **Verified only on CI / by you:** the close request, the window icon and the exe icon cannot be exercised in this
  sandbox; the page side (lock button, close answer) is covered by the browser tests.

## Feedback to agents, corrections in review, history on entries (2026-11-24)

- **Reject with a reason.** The Reject dialog offers quick reasons (Wrong, Already there, Not wanted, Not now) and an
  optional line (max 500 characters). The note is stored as `code: text` on the proposal and read by agents.
- **Resend guard.** Each proposal keeps a fingerprint of what it asked for (new people by name, sex and birth year; links
  by type and ends; edits by target, operation and new values; notes and wording do not count). Sending the same thing
  again after a rejection, from any agent, is refused with `previously_rejected`, the proposal id and the reason, unless it
  carries `"acknowledge_rejected": ["<id>"]`. The guard only fires when every item matches one rejected proposal.
- **Correct a value in review.** A pencil on every new person, link and suggested edit opens a small form (names, sex,
  dates and certainty, places, note; link kind or partnership state, dates; the new value of an edit). Corrections are
  checked like an agent's proposal and written straight to the pending records, so approving applies them. Who a link
  joins cannot be changed (reject and ask again). The agent's original value is kept in `proposal.reviewer_edits`, which
  also drives "Put back what the assistant said".
- **History on entries.** `get_person` returns `history` (newest 20 events, `history=all` / `history_all` for everything),
  `history_total` and `user_confirmed`. Events come from the revision log, the proposals and the reviewer corrections; nothing
  is stored twice for them. Events: proposed, approved, rejected, corrected, edited, edit_proposed / approved / rejected,
  removal_*, created, removed, restored. Notes are capped at 300 characters and always sit under `note`; `AGENTS.md` says
  they are information, never instructions. `search_people` adds `previously_rejected`; `list_my_proposals` adds
  `rejection_note` and `reviewer_corrections`.
- **Kept on purpose:** the `reviewer_edits` column (I had meant to derive everything from the revision log). A revision
  holds whole rows with column names, not the fields the agent used, so the agent's original value and the one-click revert
  need a small log of their own. Agents are told about it only through the derived views above.
- Known limit: history of a link shows only the link's own events; edits of names appear under the person with the name's
  label. `user_confirmed` is information for now; the server does not refuse proposals that touch those fields.

## Withdraw, duplicates and listing for agents (2026-11-24)

Prompted by an agent's review of the MCP design after a large import.

- **Withdraw / replace.** An agent can take back its own pending proposal (`withdraw_proposal`) or send a corrected one
  with `replaces`, which withdraws the old one first. Withdrawn is a flag on a rejected proposal (the status check stays
  pending/approved/rejected); withdrawn proposals do not count for the resend guard. It fails if another pending proposal
  already uses people the old one added.
- **Duplicates.** Warnings now cover people waiting in other proposals and use shared relatives (by id or name key):
  `likely_duplicate` (same name key plus a shared parent or partner, birth years not differing) and `possible_duplicate`.
- **Listing.** `list_people` (`/agent/v1/directory`) pages with filters, including loose ends (`no_parents`, ...).
  Search results are cards with parents, partners, children count, birth year and `proposal_id`; `display_name` stays for
  older clients. `get_proposal` returns one of the agent's own proposals (another agent's id is NotFound). The default
  search limit is 50. Both new tools are read-only, so the "no approve/edit/delete" rule still holds (nine tools).
- **Calendars:** see the next section.

## Calendars: one stored calendar, Jalali as input and display (2026-11-25)

- **Storage stays Gregorian.** One scale for sorting, matching, ages and the quilt. No per-date calendar column.
- **Agents send `calendar: "jalali"`** on a person, relationship or change, with the date exactly as the user gave it. The
  server converts before validating (`crates/review/src/calendar.rs`), removes the key, and returns `date_conversions`
  (given, stored, approximate, note). Only `gregorian` and `jalali` are accepted; anything else or an impossible date
  (month 13, Esfand 30 in a non-leap year) is a field error. In a change, `calendar` applies to `to`, not `from`.
- **Partial dates.** A Jalali year or month does not fit one Gregorian year or month, so it is stored at the same
  precision using its middle, with qualifier `about` (people), plus a note and an appended `agent_note` giving what the
  agent sent. Full dates are exact.
- **Algorithm.** `crates/store/src/jalali.rs`, a port of the `jalaali-js` leap table (valid for years -61 to 3177 AP). Tests
  compare every leap flag and about 3,900 sample days with that library and round-trip every day from 1700 to 2100.
- **Display.** Setting "Show dates in" (Gregorian or Jalali) changes only `fmtDate`, using the browser's Persian calendar.
- **Typing in the app:** with the Jalali setting on, the person form and the review corrections show dates in Jalali and
  convert what is typed through `POST /api/calendar/convert` before saving (a date left as shown is kept as it was; a typed
  Jalali month or year becomes `about`). `web/js/dates.js` does the showing, the server does the converting.
- **Not done:** Hijri (lunar) calendar, which depends on
  moon sighting and would need a "may be a day off" qualifier; showing the original text of partial dates in the UI.

## Agent review of list_people: fixes (2026-11-25)

An agent's review found `sex` silently matched nobody: the filter lowercased its input but the store keeps `M/F/X/U`.
Fixed and tested with every vocabulary. Also from the review:

- **Never silently empty.** Bad `sex`, `status`, `fields`, `group_by` or `format` values are errors that list the
  allowed values; unknown query parameters (HTTP) and unknown tool arguments (MCP) are errors; results echo `filters`.
  Proposals accept `male/female/other/unknown` as well as the codes.
- **Enums in the tool schema** for the query filters and for `sex`, qualifiers, link `type`, `kind` and `subtype` in
  `create_proposal`.
- **Small calls:** `fields` projection, `format=columns`, `group_by` counts, limit up to 2000, compact JSON in MCP results
  (was pretty-printed), `tree_version` on list and search results (id of the newest revision).
- **Short reading guide** at `family-quilt://guide/reading` and `/agent/v1/guide/reading`.
- Empty `list_people` results are written to the activity log with the filters.
- **Not done:** CSV output (columns covers it), ETag/`If-None-Match`, size hints, a separate stats tool (`group_by`).

## Cursor paging replaces offsets (2026-11-26)

`list_people` takes `after` (the previous `next_cursor`) instead of `offset`; offsets are gone (development phase, no old
callers to keep). A cursor is `<tree_version of the first page>~<last id>`, so a stateless agent only needs the previous
answer. People are sorted by id, so a change earlier in the list cannot shift what comes next. Later pages carry
`changed_since_start` (the tree changed since the first page), which the agent cannot work out alone. Results also show
`remaining_after_this_page`. Not built: `if_version` ("unchanged" replies), ETag headers.
