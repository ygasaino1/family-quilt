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

**Rendering.** World units: 20 x 20 px cells, header cells above each column above row 0. Rows and columns are index-addressed, so only the visible range is visited (binary search inside a column). Shapes are batched into one path per style. Zoom levels: below ~1.2 px per column cells are skipped (only lines remain); below ~4 px they are plain blocks; labels are greeked below 8 px and hidden below 4 px. Symbols follow the mock-up: square = man, circle = woman, diamond = other/unknown; filled = parent or birth child; hollow outline = non-birth child; pending = dashed amber outline with hatching, a badge and an amber dot before the name; header letters come from the translation table (M, U, D, ?).

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
