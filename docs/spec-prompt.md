# Prompt: GeneaQuilts-based family tree on Rust + SQLite, with adopted/foster/step children

You are a senior engineer building a real-genealogy system. Work autonomously, in phases, and stop at the marked gates to report and wait for my decision. Prefer boring, well-tested choices. Do not invent facts about the GeneaQuilts codebase: read it and quote file paths for every claim.

**Start with Phase 0 only, then Phase 0.5. Do not begin Phase 1 until I have answered Gate A and Gate C.** Later sections are the full specification; read all of it first, but build in the order of the phase table in section 13.

---

## 1. Goal

A local desktop application (embedded web UI in its own window, see section 15) for a real family's genealogy (real people, private data) that:

1. Uses the **GeneaQuilts** visualization (rows = individuals, columns = nuclear families, diagonally filled matrix) as the main view, with its overview, timeline, search/filter and "Bring & Slide" navigation as far as they are feasible.
2. Stores data in **SQLite** and serves it from a **Rust** backend.
3. Adds what GeneaQuilts (as far as we know) lacks: **non-biological parent–child links** (adopted, foster, step, guardian, …), marked with **distinct symbols in the family column**, and handled correctly in relationship-finding.
4. Is **AI-agent friendly**: an agent can add people as *pending*; I review and approve them in the UI.
5. Answers "how are A and B related?" fast on large trees, including marriage links.

Data is entered manually by me or by an agent. Names can be in Persian or any script; the UI must handle right-to-left text.

## 2. What is known about GeneaQuilts (verify all of it in Phase 0)

- Repository: https://github.com/jdfekete/geneaquilt (Java, Maven, ~31 commits at last look). Project page: https://aviz.fr/geneaquilts. Paper: Bezerianos, Dragicevic, Fekete, Bae, Watson, "GeneaQuilts: A System for Exploring Large Genealogies", IEEE InfoVis 2010.
- Rows are individuals, columns are nuclear families. Input is GEDCOM (.ged), with experimental QTDT (.ped). To view your own file it needs **Graphviz** to produce layer (`.lyr`) files. Targets genealogies up to several thousand individuals.
- **I could not confirm its license**: the GitHub page shows only the word "License". Finding the actual license is your first task, because it decides whether you may port code.
- It is a Java desktop program. A Rust backend cannot extend it directly. Do **not** try to wrap or embed the Java app. "Using GeneaQuilts as the base" therefore means: reuse its visual design, interaction ideas and layout algorithm (ported or re-implemented, see the license gate), and keep GEDCOM import compatible with it.

## 3. Phase 0 – Investigate and decide (no product code yet)

1. Clone the repo. Read `LICENSE`/`pom.xml`/README, list the source packages, and identify: the data model, GEDCOM loading, the generation-layering step (and exactly what it asks Graphviz to do), row/column ordering, the rendering code and its visual encoding (what each cell colour/shape means), and the interactions.
2. Write `docs/geneaquilts-analysis.md` with: license text and its implications; a **complete feature inventory** of the original (every menu item, mouse/keyboard interaction, view, filter, export and file format it supports, found by reading the code **and by building and running it**: `mvn install`, then `run.sh`/`run.bat`, using its own sample files) as a table "GeneaQuilts feature → how we deliver it → status"; the layering/ordering algorithm in pseudocode; the visual encoding exactly as implemented; any limitations around non-biological relationships (search the code for `PEDI`, `ADOP`, `FOST`, and for how a child with more than one family is treated).
3. **Marriage question (answer explicitly in the analysis doc):** how does the original quilt show whether the two parents in a family column are married, unmarried, divorced, or only co-parents? Check how the GEDCOM loader treats `FAM`, `HUSB`/`WIFE`, `MARR`, `DIV` and a family with no marriage event, and what the renderer draws for it (marks, lines, colours, header text). If it does not distinguish them, say so plainly. Also check how it shows a person with several marriages and a couple without children.
4. Find the paper (open version if available) and note where code and paper differ.
5. **GATE A – stop and report to me**:
   - License allows porting? If yes: port the layout logic to Rust with attribution and a NOTICE file. If unclear or incompatible: **clean-room** re-implementation from the paper and from observed behaviour, and say so in the docs.
   - Proposed layering approach without Graphviz (preferred: implement in Rust or TypeScript; no external binaries at runtime).
   - Any surprise that changes the plan below.

## 4. Architecture (propose changes at Gate A only if you find a real problem)

- **Backend**: Rust (stable, edition 2021), `axum` + `tokio`, `rusqlite` with the `bundled` feature (no system SQLite dependency), `serde`, `uuid` (v7), `thiserror`, `tracing`. Embedded schema migrations driven by `PRAGMA user_version`. No ORM. Keep dependencies few and justify each in the README.
- **Frontend**: TypeScript + Vite, no UI framework needed (small modules are fine). The quilt is drawn on **Canvas 2D** (or WebGL if you can show Canvas cannot hold 60 fps at 10 000 rows); only visible rows/columns are drawn; labels appear above a zoom threshold.
- **Serving**: the Rust binary serves the built frontend and the API. One process, one SQLite file. Dev mode: Vite dev server proxying to Rust. Distribution rules (single app, own window, non-technical users) are in section 15.
- **Security**: bind to `127.0.0.1` only (this is private family data). Two doors, as below.

### Two doors (required)

- `/agent/v1/*`: bearer token (generated once, stored in `data/agent-token.txt`, overridable by env). `X-Agent-Name` header → `created_by = "agent:<name>"`. Allowed: guide, search, read person, relationship query, graph read, **create proposal** (which may contain proposed edits to existing records, section 19), list own proposals. **Not allowed**: direct edit, direct delete, approve, reject. These routes must not exist.
- `/api/*`: for the browser UI only. Reject unless `Sec-Fetch-Site` is `same-origin` or `none` **and** the `Host` header is `127.0.0.1[:port]` or `localhost[:port]`. This blocks scripts and other websites. Test it with raw HTTP (clients like `reqwest` set `Host` themselves).

## 5. Data model (SQLite, flat, normalized)

Use UUIDv7 text ids (strictly increasing within a process: add a counter for the same millisecond). Soft deletes (`deleted_at`). Timestamps in UTC ISO-8601. Create all tables in migration 1; later changes are numbered migrations.

```sql
person(id PK, sex CHECK IN ('M','F','X','U') DEFAULT 'U',
       is_living INT, birth_date, birth_qualifier, birth_place_id,
       death_date, death_qualifier, death_place_id,
       status CHECK IN ('verified','pending') DEFAULT 'verified',
       vault_id DEFAULT NULL,   -- NULL = public; reserved, see section 16
       proposal_id, created_by, agent_note, merged_into, created_at, updated_at, deleted_at)

person_name(id PK, person_id, type CHECK IN ('birth','married','alias','religious','other'),
            given, family, full_name, script, is_primary, sort_order)   -- many names, any script

relationship(id PK, type CHECK IN ('parent_child','partnership'),
             person_a, person_b,            -- parent_child: a = parent, b = child
             kind,                          -- parent_child only, see below
             subtype,                       -- partnership: married|unmarried|divorced|ended|unknown (divorced = legal end of a marriage; ended = an unmarried partnership that ended; both show as D)
             start_date, end_date, place_id, note,
             status, vault_id DEFAULT NULL, proposal_id, created_by, agent_note, created_at, deleted_at,
             UNIQUE(type, person_a, person_b))

event, place, source, citation       -- as in a normal genealogy model; screens come later
                                     -- (every one of these tables also has vault_id DEFAULT NULL)
proposal(id PK, summary, source_text, created_by, status CHECK IN ('pending','approved','rejected'), created_at, decided_at)
meta(key PK, value)
```

### The new part: parent–child `kind`

`relationship.kind` for `type='parent_child'`, `NOT NULL DEFAULT 'birth'`, `CHECK IN`:

| kind | meaning | GEDCOM `PEDI` | counts as blood |
|---|---|---|---|
| `birth` | biological parent | `birth` (or no PEDI) | **yes** |
| `adopted` | legal adoption | `adopted` | no |
| `foster` | foster care | `foster` | no |
| `step` | step-parent *recorded explicitly* | (none; export as a NOTE) | no |
| `guardian` | legal guardian / raised by | `other` (7.0) | no |
| `sealing` | religious sealing | `sealing` | no |
| `unknown` | link exists, nature not recorded | none | **treated as unknown, not blood** |

Rules:
- Step-parents are normally **derived** (partner of a parent). Store `step` only when I explicitly record it; never create both a derived and a stored one.
- A child may have several parent links of different kinds (birth parents + adoptive parents). That is normal and must work everywhere.
- Do not add columns I did not ask for (no `kind_certainty` for now). Use the short `note` for oddities.
- Constraints: no cycles in parent_child (check on insert, in a transaction); a person cannot be their own ancestor; a link needs both people to exist and not be deleted.
- Fuzzy dates: `1887`, `1887-04`, `1887-04-12`, with qualifier `exact|about|before|after`. Sort/compare using the earliest possible day.

### Staging (pending/verified)

- Anything created by an agent is `pending`, along with its links; a link is effectively pending if it or either endpoint is pending.
- `proposal` groups one agent request. A proposal is created **atomically**; people in it can be referred to by local `ref` names so links can point to people created in the same proposal.
- Pending data is **excluded** from verified answers (relationship finding, default graph) unless `?pending=1`.
- Humans can approve or reject a whole proposal. Approve is blocked if its links point to people pending in another proposal. Rejecting removes its people and links. An empty proposal is auto-rejected.
- Computed warnings shown on review (not stored): possible duplicate person (same normalised name + overlapping dates), duplicate inside the proposal, implausible parent age (< 12 at child's birth), parent died before child's birth (allow ≤ 9 months for the father), death before birth, more than 2 `birth` parents, person with no links, a child with two `adopted` parents-sets that overlap in time.

## 6. The quilt: how families become columns

GeneaQuilts needs "nuclear family" columns. The database does not store families; **derive them** in Rust (a pure, heavily tested function) and cache per request.

**Family column definition.** A column is a set of parents (1 to 2, rarely 3+) and the children linked to that set, plus the partnership between the parents if one exists.

Derivation, per child:
1. Group the child's parent links by *kind class*: `{birth, sealing}` = biological class; each of `adopted`, `foster`, `step`, `guardian`, `unknown` is its own class.
2. Each group becomes a candidate column keyed by its sorted parent ids.
3. **Merge rule**: two groups of the same child merge into one column when their parents are partners of each other (an existing `partnership`) **and** the merged set has at most 2 parents. Example: birth mother A + adoptive stepfather C, with A and C partnered → one column {A, C}; the child's cell shows the symbol for the non-birth link (per-parent detail in the tooltip).
4. Otherwise the child appears in **several columns** (birth family and adoptive family): one cell per column. This is the point of the feature: the row shows both families.
5. A partnership with no children still gets a column (a couple column). A single parent with children gets a **single-parent column** (no placeholder partner is needed in the quilt; this replaces the "unknown partner" idea from my earlier tree view).
6. Column key = sorted parent ids. Two children with the same key share a column. Children with different non-merged keys never share one.

Write the derivation as `fn derive_families(people, links) -> Vec<Family>` with a table-driven test file covering at least: nuclear family; single parent; same parents with one birth and one adopted child; birth + adoptive families for one child; birth mother + adoptive stepfather partnered; adopted by two unpartnered people; foster then returned to birth family; three parents (flag with a warning, still render); half-siblings; child with only a `step` link; pending links; partnership with no children; remarriage (two couples sharing a parent).

**GATE B – stop and show me the derivation rules as a table with the test cases before building the renderer.** I may change the merge rule.

## 7. Symbols for non-biological children (decided)

Decisions already made by me; do not re-open them:
- **One column per parent set.** A non-blood child sits in the same column as the other children of those parents. There are **no separate "F"/"H" column types and no computed F / H / F+H column badge.** Do not split a couple into two columns by blood status.
- **Only two new marks, no colours, no per-kind shapes.** GeneaQuilts uses a filled black square for a blood-related male and a filled black circle for a blood-related female (verify this in Phase 0 and keep the original encoding exactly as the code defines it). Add:
  - **outlined (hollow, black outline) square** = non-blood-related male child
  - **outlined (hollow, black outline) circle** = non-blood-related female child
  - For sex `X`/`U`, reuse whatever the original does for unknown sex, drawn outlined.
- In Phase 0, check that the original does not already use outlined shapes for something else. If it does, stop at Gate A and propose an alternative.
- **Which link kinds are drawn outlined:** every kind except `birth`: `adopted`, `foster`, `step`, `guardian`, `sealing`, `unknown`. `unknown` is outlined because it cannot honestly be shown as blood; its tooltip says "nature of link not recorded".
- The exact link kind is shown in the **hover tooltip** ("adopted by …", "foster child of …"). It may later be removed from the tooltip; keep the tooltip text in one small function so it is easy to drop.
- Clicking a person opens the **sidebar with their full data**, including every parent link and its kind (editable there). This is where the detail lives, not in the quilt.
- A person who is a child in more than one column (birth family and adoptive family) is highlighted across all their cells on hover/selection, with a connector line between their cells.
- **Lineage highlighting/tracing** has two modes: *blood lineage* follows `birth` links only (it stops at outlined cells); *family lineage* follows all links. Default to blood lineage, with a toggle.
- Pending items: amber hatched fill plus a small badge, independent of the blood/non-blood outline.
- **Column header letter = the couple's state (decided).** Where the original quilt puts its "F" (family) marker, show a one-letter partnership state instead: **M** married, **U** unmarried (partners living together), **D** ended (divorced, or an unmarried partnership that ended; the tooltip says which word applies), **?** partnership recorded with unknown state, **P** column with no recorded partnership (a single parent, or co-parents with no partnership record; "P" = parent/partner). There is no separate "separated" state. GEDCOM import maps `DIV` to `divorced` and any separation/ended marker to `ended`. A recorded partnership always gets its own column, even with no children. Hovering the header shows the full word and details ("Married 1995 · Divorced 2010 · Tehran"). No widowed letter: a marriage ended by death stays **M**, and the tooltip may say "ended by death of X". No dimmed headers or other extra cues for ended partnerships. The letters come from the translation table (section 20), never hard-coded in the renderer.
- **Deceased people get a gray background on their name** (row label), for everyone who is deceased (`is_living = false` or a death date exists), not only widowed people. It must keep readable contrast in light and dark themes, and must not rely on colour alone where the name is also shown with dates in the sidebar. In privacy mode it still applies. Check in Phase 0 whether the original already uses a gray background for something else.
- Dark and light themes via CSS variables / a token object. Provide a small legend (filled vs outlined) that can be hidden.

**GATE B includes a mock-up PNG** of a quilt with: a nuclear family; a couple with one birth and one adopted child (same column); a child shown in a birth column and an adoptive column; a single-parent column.

## 8. Relationship finding ("how are A and B related?")

Port this behaviour (it is implemented already in my Node prototype, see §12):

1. **Blood relation**: closest common ancestor over `birth` links **only**. Name it exactly: sibling, half-sibling (one shared birth parent), nth cousin, nth cousin m times removed, great-grand-…, aunt/uncle, niece/nephew. Report the common ancestors and both path lengths.
2. Otherwise the cheapest route (Dijkstra) over up / down / partner steps. Costs: parent or child step 1; partner step 1.5 (+0.5 if the partnership ended); a non-birth parent/child step costs the same as birth but is flagged. Label by pattern: in-laws, step-relatives, adoptive parent/child/sibling ("adoptive brother"), foster parent/child, otherwise a readable chain "A → mother of → … → B".
3. Output includes `by_blood: bool`, the path with each step's kind, and `includes_pending`.
4. Pending people/links are ignored unless `pending=1`; then flag `waiting_for_review`.
5. **Performance**: build the graph in memory (adjacency lists keyed by integer indexes, loaded lazily and invalidated on write) or use recursive CTEs, and benchmark. Target: p95 < 50 ms on a 100 000-person synthetic tree; bidirectional search for the generic route.
6. This fixes a known flaw of my prototype: it treated adopted/step/foster links as blood.

## 9. UI beyond the quilt (design from this mock description)

- Left **icon rail**: Home (quilt), People, Review (badge with pending count), Agent access, Settings.
- Right or left **PROFILE sidebar** (as in my mock): avatar with initials; First name, Last name, Gender; Birth date + place, Death date + place, Living checkbox; **Remove**; **Save changes**; buttons **Add parent / Add spouse / Add child / Add sibling**; **How are we related?**; **View full profile**; recent searches.
- **Add parent / Add child** dialogs include a **link kind** selector (default birth), with the symbol shown next to each option. Editing an existing link's kind is possible from the profile's relatives list.
- Canvas controls: zoom −/+, Fit, a small overview (minimap), a timeline strip (as in GeneaQuilts), search box with result list.
- **Review screen**: list of pending proposals (agent name, summary, your original words next to what the agent understood), each person/link editable or removable, warnings shown inline, **Approve all / Reject all**. While reviewing, the quilt shows only that proposal's people and their direct relatives.
- **Selecting a person must not move the canvas under the user's eye.** Keep the clicked row/cell at the same screen position when the layout changes; only pan to centre when the person was off-screen or came from search. Prefer short animated transitions (≤ 250 ms) for layout changes.
- **Privacy mode**: living people shown as initials, no dates.
- Poll for new proposals every 5 s while the tab is visible; toast on arrival.
- Settings: tree title, privacy mode, **backup download** (`VACUUM INTO` a temp file, stream it).

## 10. Agent API (v1) and guide

- `GET /agent/v1/guide` returns plain text instructions for an agent: how to search before proposing (avoid duplicates), how to express links, what `kind` values mean and when to use them ("adopted" only when the user says adopted; if unsure use `unknown` and add `agent_note`), dates format, that nothing is final until the human approves.
- `GET /agent/v1/people?q=`, `GET /agent/v1/people/{id}`, `GET /agent/v1/relationship?a=&b=`, `GET /agent/v1/graph?focus=&up=&down=`, `POST /agent/v1/proposals`, `GET /agent/v1/proposals`.
- Proposal body: `{summary, source_text, people:[{ref, given_name, family_name, sex, birth_date, ...}], relationships:[{type, a, b, kind?, subtype?, ...}]}` where `a`/`b` are either existing ids or `ref`s. Validate strictly; return field-level errors an agent can act on.
- Also write `AGENTS.md` with the same guide. MCP access is specified in section 18.

## 11. GEDCOM

- Import (GEDCOM 5.5.1; 7.0 where easy): INDI, FAM, NAME (with GIVN/SURN), SEX, BIRT/DEAT (DATE with ABT/BEF/AFT, PLAC), MARR/DIV, `FAMC` with `PEDI` mapped to `kind` (`adopted`, `foster`, `birth`, `sealing`; unknown values → `unknown` plus a note). A child with several `FAMC` links imports as several parent sets.
- Export the same back, including `PEDI`. `step`/`guardian` export as `PEDI other` where 7.0, otherwise a NOTE.
- Import is a **proposal** (so I can review it), size-limited and streaming; reject malformed files with line numbers.
- Provide `tools/gen_gedcom` (or a Rust bin) that generates a synthetic tree of N people (including adoptions, fosters, remarriages, half-siblings, cousins marrying) for tests and benchmarks.
- Use GeneaQuilts' own sample files to check that the import produces the same families they do.

## 12. Reference implementation (if attached)

I may attach `family-tree.zip`, a working Node 22 + SQLite prototype of the same system. If present, read first: `AGENTS.md`, `server/schema.sql`, `server/db.js` (migrations), `server/store.js`, `server/proposals.js`, `server/pathfinding.js`, `server/labels.js`, `server/app.js` (two doors), and `test/*.test.js`. **Port behaviour, not code style**, and keep its tests as a behaviour checklist (translate them to Rust tests). Known gaps in the prototype that you must fix: adopted/foster/step counted as blood; single-parent children drawn from the parent's card; the view re-centring on every click.

## 13. Phases, gates, acceptance

| Phase | Deliverable | Acceptance |
|---|---|---|
| 0 | `docs/geneaquilts-analysis.md` | **GATE A** |
| 0.5 | Front-end spike (section 15) | **GATE C** |
| 1 | Cargo workspace, SQLite (+ optional SQLCipher password, section 16) schema + migrations, ids, fuzzy dates, store layer with access context, tests | `cargo test` green; cycles/dup links rejected; migrations idempotent; section 16 acceptance |
| 2 | `derive_families` + tests, GEDCOM import/export with PEDI, synthetic generator | table-driven tests pass; round-trip import→export→import is stable; **GATE B** |
| 3 | Relationship finding with kinds, benchmarks | blood/non-blood correctness tests; p95 < 50 ms on 100k |
| 4 | HTTP: both doors, proposals, approve/reject, warnings, **agent edits as proposals (section 19, incl. `change` and `revision` tables)**, **MCP (section 18)** | HTTP tests incl. forged `Host`/missing `Sec-Fetch-Site`, agent cannot approve; section 18 and 19 acceptance tests |
| 5 | Quilt renderer + symbols + header letters + sidebar + review UI + **translation tables (section 20)** | Playwright e2e: select without view jump, symbols visible, pending styling, review flow; 60 fps pan/zoom on a 10k-person file; screenshots light/dark |
| 6 | Docs, polish, packaging | **GATE D (parity, section 21)**; one-command run; README with architecture and decisions |

Definition of done for every phase: tests added and passing, `cargo fmt`, `cargo clippy -- -D warnings`, no new dependency without a line in the README explaining why, a short note of decisions and open questions at the end of your message.

## 14. Working rules

- Ask me before: implementing per-vault encryption or any multi-user feature, choosing a license for new code, adding a runtime dependency on a non-Rust/JS binary, adding any further symbols or colours, reintroducing F/H column types, changing the family-derivation rule, anything that sends data off this machine.
- No telemetry, no external network calls at runtime, no CDN assets (the app must work offline).
- Never hard-delete real data: soft delete, and a backup download.
- Do not put real family data in tests or in git. Use synthetic data only.
- Report briefly at each gate: what is done, what you verified (commands and results), what you need from me. Do not narrate every step.
- If something in this prompt conflicts with what you find in the GeneaQuilts code, say so at Gate A instead of guessing.

## 15. Packaging and users (required)

The end users include **elderly, non-technical family members**. The product is **one app they double-click**.

- **Single executable**: the web UI files are embedded in the binary (e.g. `rust-embed`/`include_dir`), SQLite is compiled in (`rusqlite` `bundled`). No installer prerequisites besides what the OS already has. **Do not bundle a browser engine** (no Electron/Chromium).
- **Own window by default**: use the OS web view (`wry`/Tauri-style: WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux). The user never sees a browser tab, a terminal, a port number or a token. A `--browser` flag opens the default browser instead, for development and as a fallback if the web view is unavailable. If WebView2 is missing on Windows, show a plain-language message with a link, not a crash.
- Pick a **free localhost port automatically**; enforce a **single running instance** (a second launch focuses the first window). Keep the localhost-only binding and the two-door security model from section 4; the window uses the same-origin rules.
- **Data location**: the platform's standard app-data folder by default, or a `data/` folder next to the exe if a `portable` marker file exists. First run is a small wizard: tree name, where to keep the data, optional import of a GEDCOM file.
- **Backups**: automatic daily/weekly rolling backups (`VACUUM INTO`, keep the last N) plus a one-click "Back up now / Restore". Never lose data on a crash (WAL mode, transactions).
- **Accessibility**: a text-size setting (and Ctrl +/− zoom), high contrast, large click targets (≥ 44 px) in forms and the sidebar, keyboard navigation, plain-language labels and error messages (no jargon, no HTTP codes), confirm before destructive actions, undo for the last edit where feasible. The quilt must stay usable with a mouse wheel and on-screen −/+ buttons.
- **Agent features out of the way**: the agent token, API address and instructions live under Settings → "AI assistant (advanced)". They must not appear in the main flow.
- **Portability**: no Windows-only code in the core; use the `dirs` crate (or equivalent) for paths; design so Windows, macOS and Linux can each be built later (see section 17: Windows only for now); one artifact per OS when the time comes. Document code signing as a release step: unsigned Windows exes trigger SmartScreen warnings and unsigned macOS apps are blocked by Gatekeeper, which are real obstacles for non-technical users. Do not buy certificates or accounts; list what I would need to do.
- **Offline**: no network calls and no CDN assets at runtime. No auto-update in v1; show the version number in Settings.
- **Sharing between family members is out of scope for v1** (each copy has its own database file). Make export/import of the whole tree (a single file) easy, and note in the README that real-time sync is a separate future project.

### Phase 0.5 – front-end spike (before Phase 1)

Prove the stack with measurements before building on it: a tiny embedded-UI binary that opens its own window and draws a **synthetic 10 000-person quilt on Canvas 2D** with pan/zoom and viewport culling, plus a row of **Persian and mixed RTL/LTR names**. Report frames per second while panning and zooming, binary size, startup time, and screenshots, on the OS you are running. If Canvas cannot hold 60 fps, say so and propose WebGL before continuing. **GATE C – show me the numbers.**

## 16. Database protection (decided scope)

**Version 1 = optional password for the whole database file, KeePassXC-style. No per-user or per-vault encryption yet.**

- Use **SQLCipher** through `rusqlite` (the `bundled-sqlcipher` family of features; check which variant builds cleanly on Windows, macOS and Linux without a system OpenSSL and report it). Encryption is **optional**: a tree can be unprotected or protected, chosen in the first-run wizard and changeable in Settings (set, change and remove password).
- Key handling: strong KDF settings, key kept only in memory while unlocked and zeroed afterwards (`zeroize`), never logged. A protected file must not be readable by plain SQLite (its header must not be `SQLite format 3`); test this.
- **Unlock screen** at start. While locked, `/api/*` and `/agent/v1/*` refuse everything with a clear "locked" error (agents must not be able to bypass it). Optional auto-lock after N minutes idle.
- **Wrong password** gives a plain-language message and a short delay, no crash and no corrupted file.
- **Lost passwords are the biggest risk for non-technical users.** Propose, at Gate A, a recovery design (for example: the database key is random and stored wrapped twice in a small header, once by the password and once by a generated **recovery key** that the wizard shows once and asks the user to print or save). The wizard must warn clearly that without the password or the recovery key the data cannot be recovered.
- **Backups and exports stay protected**: automatic and manual backups of a protected tree are protected the same way (verify that the `VACUUM INTO` path keeps encryption; if not, use the SQLCipher export method). Plain GEDCOM export is a deliberate, confirmed action with a warning that the file is not encrypted.
- Changing the password re-keys the file (`PRAGMA rekey` or equivalent) atomically, with a backup taken first.

**Reserved for later (do NOT implement now): per-vault encryption.** What to do now so it fits later without a rewrite:
- Every record table has `vault_id`, `NULL` meaning **public** (this is what all v1 data is). Nothing in v1 sets it to anything else; no `vault` table yet.
- All reads and writes go through the store layer, and every store function takes an **access context** argument (v1: "everything", the only value that exists). Do not scatter SQL outside the store layer; the later feature needs one place to filter and encrypt.
- Keep the layout/derivation/pathfinding code operating on in-memory graphs built by the store layer, so hidden data can later simply be absent from them. Design the relationship output to be able to carry an "incomplete, some data is private" flag (always false in v1).
- Document these seams in `docs/future-vaults.md` (one page): the idea (a vault per owner group, each with its own random key wrapped per authorised user), the open problems (links between private and public people, answers through hidden people, revoking access, no enforced write permissions in a plain file, leaked metadata), and the fact that multi-user editing is a separate sync/server project.

Acceptance (add to Phase 1 and Phase 4): create protected tree → close → reopen needs password; wrong password rejected; file not readable by the plain `sqlite3` CLI; locked state blocks both doors; backup of a protected tree is protected; unprotected trees behave exactly as before; `vault_id` exists on all tables and is NULL for all data.

## 17. Platform focus (decided)

- **Develop, test and ship for Windows 10/11 (x64) first.** All "verify on Windows, macOS and Linux" statements above mean: **Windows is the only platform you must build and test on now.** Do not spend time on macOS/Linux builds, signing or packaging in this project phase.
- **Cross-platform is still the target**, so keep the core portable: no Windows-only APIs outside one small `platform` module (window/web view creation, app-data folder, single-instance lock, opening files), standard path handling (`PathBuf`, `dirs`), no hard-coded `\\` separators, no assumptions about line endings or case-insensitive paths. Put anything OS-specific behind that module so other systems can be added by implementing it later.
- Windows specifics to handle: WebView2 presence check with a friendly message and link if missing; app data under `%APPDATA%` (or next to the exe in portable mode); long-path and non-ASCII path safety (user names and folders may contain Persian characters); Windows Defender/SmartScreen note in the README for the unsigned exe; build with the MSVC toolchain; `cargo build --release` must produce the single exe with no extra DLLs (statically link the C runtime and verify with a dependency check).
- Verify the SQLCipher variant (section 16) builds cleanly **on Windows without a system OpenSSL**, and report the exact crate feature used.
- Keep a short `docs/porting.md` listing what is expected to differ on macOS and Linux (web view engine, app-data folder, signing) so adding them later is a checklist, not a research task.

## 18. MCP integration (built in, not a late add-on)

Agents must be able to use the tree **while the app is open**, through MCP, with no extra program to install.

- **Two entry points in the same exe, both thin layers over the existing agent API (section 10), so they inherit its limits:**
  1. **Embedded MCP endpoint** (Streamable HTTP) on the app's localhost server, e.g. `/agent/mcp`, bearer-token protected exactly like `/agent/v1/*`.
  2. **Bridge mode**: `app.exe --mcp-stdio` runs as a stdio MCP server that forwards every call to the *running* app's localhost agent API (reads the port and token from the app-data folder). Agent apps that launch local MCP servers from a config file use this. If the app is not running, return a clear tool error ("Open the Family Tree app first"), never start a second copy of the database.
- Before building, **check the current MCP specification and the current docs of the clients I use (Claude Desktop, Claude Code) for which transports they support for local servers**, and tell me the result at Gate A. Do not assume; adjust the two entry points if needed.
- **Tools** (names are suggestions): `search_people`, `get_person`, `find_relationship`, `get_graph`, `create_proposal`, `list_my_proposals`. Resource or prompt: the agent guide text. Tool descriptions must tell the agent to search before proposing, to use the link `kind` values correctly, and that nothing is final until I approve.
- **There must be no approve, reject, direct edit or direct delete tool** (edits exist only as proposals, section 19), and no code path from MCP to those actions. Test this: list the MCP tools and assert it; call the browser-only routes with the agent token and assert they fail.
- Locked file or missing token: tools return a plain-language error. The agent name comes from the MCP client name (or an `agent_name` setting) and is stored as `created_by = "agent:<name>"`.
- Proposals made through MCP appear in the Review screen in real time (the 5 s poll or a push), with the same warnings.
- The **Settings → "AI assistant (advanced)"** screen shows ready-to-copy config snippets for the clients above (command path of the exe for bridge mode, URL and token for the endpoint) and a "Test connection" button that lists the tools and shows the last agent activity.
- Move this work into **Phase 4** (HTTP and doors) instead of Phase 6; Phase 6 keeps only polish and packaging. Acceptance: an MCP client can search, read, ask for a relationship and create a proposal while the app is open, and the proposal shows up as pending in the UI.

## 19. Agent edits of existing records (proposals only)

Agents may *propose* changes to existing, verified records. A change never touches the record until I approve it.

- **Where**: the same `POST /agent/v1/proposals` (and the MCP `create_proposal` tool) accepts a `changes` array next to `people` and `relationships`, so one proposal can mix new data and edits.
- **Allowed operations**: `update` fields of a person (names, sex, birth/death date, qualifier, place, living flag, `note`); `update` a name entry; `update` a relationship (`kind`, `subtype`, dates, `note`); `remove` a relationship. **Not allowed**: deleting a person, merging people, and editing `id`, `status`, `vault_id`, `created_by`, `proposal_id`, `merged_into`, timestamps (whitelist the editable fields; reject anything else with a field-level error). Deleting or merging people stays a human action for now.
- **Storage**: a `change` table (`id, proposal_id, target_type, target_id, op, patch JSON {field: {from, to}}, base_updated_at, agent_note, status`). The proposal carries the `from` values the agent saw. The target record is **not modified** while the proposal is pending.
- **Staleness / conflicts**: at review and at approval, compare each `from` with the current value (or `base_updated_at` with the record's `updated_at`). If they differ, mark the change **conflicting**, show *original / current / proposed* side by side, and never apply it silently; I choose per field.
- **Review UI**: each change shows *before → after* per field with its own accept/reject checkbox, grouped under the person or link. Approving a proposal applies all accepted changes and new records in **one transaction**; rejecting leaves everything as it was. A change whose target was deleted meanwhile is marked "target no longer exists".
- **Computed warnings run on the resulting state** (what the data would look like after approval): implausible parent age, death before birth, parent died before child's birth, cycles, and an **impact note** for link-kind edits, e.g. "changing this link to adopted removes N people from X's blood lineage and changes N stored relationship answers" (compute the number of affected relationship results or at least the affected people for small trees).
- **Display while pending**: the person's card/row keeps showing the current verified data, with a small "suggested changes" marker; the sidebar lists the pending changes for that person. Relationship finding and the default graph use **verified data only** unless `pending=1`, in which case pending changes are applied on top and the result carries `includes_pending`.
- **History and undo**: every applied change (from a human edit or an approved proposal) writes a `revision` row (`id, target_type, target_id, before JSON, after JSON, by, at, proposal_id`). The sidebar has a "History" list and **Undo** for the latest changes (undo is itself recorded, never deletes history).
- **Human edits** go through the same `revision` log so history is complete.
- An agent cannot edit records that are themselves still pending (they belong to an open proposal); it submits a new proposal instead, which I can review alongside.
- **Agent guide text** must say: propose the smallest change that fixes the problem, include why in `agent_note`, search first, and never assume an edit was applied until the proposal is approved.
- **Tests**: with only the agent token, verified data cannot be changed directly; a proposed change leaves the record unchanged until approval; approval applies exactly the accepted fields; rejection changes nothing; a stale `from` is flagged as a conflict; the impact note appears for a birth→adopted change; undo restores the previous values; disallowed fields and person deletion are rejected.

## 20. Language and translation tables

- **v1 ships English only, but every user-visible string goes through a translation table from day one**: `locales/en.json` (keys → strings), loaded by the front end, with a `t(key, params)` helper. No hard-coded UI text, including the quilt header letters (M, U, D, ?, P), tooltip words, legend text, warnings, error messages from the backend (return stable error codes plus params, and translate in the front end) and the agent guide stays English.
- **Settings → Language** selects the app language; the choice is stored in the app settings. Adding a language later must be just a new file in `locales/` (plus optional per-language overrides for date format and number format), with no code change.
- Fall back to English for missing keys and log them in dev mode. Add a test that fails if a key used in the code is missing from `en.json`, and a pseudo-locale (e.g. `qps` with accented/longer strings) to check that layouts survive longer text.
- Design for **right-to-left languages** now: use CSS logical properties (`margin-inline-start`, etc.), `dir` set from the language, canvas text drawn with correct bidi shaping, and do not mirror the quilt matrix itself (generations stay top-to-bottom; decide left-to-right order with me when a RTL language is added). Person names always use `dir="auto"` regardless of app language.
- Dates are **stored** as ISO (Gregorian, with the fuzzy-date qualifiers) and only **formatted** per language. A Persian (Jalali) calendar display is a possible future locale feature; do not implement it now, but keep all date formatting in one module.
- The translation table for column-header letters is one object per language, for example `{"partnership": {"married": "M", "unmarried": "U", "divorced": "D", "ended": "D", "unknown": "?", "none": "P"}}`, with the full word for each next to it for the tooltip.

## 21. Feature parity with the original (hard requirement)

My implementation **must not have any shortcoming compared with the official GeneaQuilts**: everything the original can do, ours can do, plus what this prompt adds.

- The Phase 0 feature inventory is the **parity checklist**. Every row must end as *delivered* (same or better), or as a **deviation that I approved in writing at a gate**. The agent may not decide on its own to drop, simplify or postpone an original feature; "drop" is not a valid status without my approval.
- Use the original as the **reference implementation**: build and run it (Java and Maven are allowed as dev tools only, never as a runtime dependency), keep screenshots of it next to ours for each view and interaction, and compare behaviour (layout order of rows and columns, navigation, search, filters, timeline, overview, Bring & Slide) on the same sample GEDCOM files.
- **Code reuse follows the license** (Gate A): ported with attribution if allowed, otherwise clean-room from the paper and observed behaviour. In the clean-room case, whoever reads the original source for behaviour must write specifications and test cases, and the implementation is written from those, so the result stays defensible.
- **Parity tests**: for the sample files that ship with the original, check that our derived families, row order per generation and column order match the original's (or document each difference and why). Add golden-file tests for the layout, and a side-by-side screenshot report in `docs/parity.md`.
- **Gate D – parity review before Phase 6**: show me the checklist with every row's status and the side-by-side screenshots.

## 22. Additional reference material (allowed, with license discipline)

The original repository may not contain everything needed. In particular, the original delegates **generation layering** to Graphviz (it needs `.lyr` files produced by Graphviz), so the ranking algorithm may live outside the repo. Use these sources, in this order of trust, and say at Gate A which of them you actually needed:

1. The original repository (code, sample files) and the GeneaQuilts paper (Bezerianos et al., InfoVis 2010).
2. For layering: the published algorithm Graphviz `dot` uses for ranking (Gansner, Koutsofios, North, Vo, "A Technique for Drawing Directed Graphs", IEEE TSE 1993: network-simplex ranking), and Graphviz's own source/docs. Graphviz must **not** be a runtime dependency; implement the ranking in our code and test it against Graphviz output on the sample files (Graphviz as a dev-only oracle).
3. The Gramps **QuiltView** addon, a second implementation of the quilt in Python: https://github.com/gramps-project/addons-source/blob/8698d46e73447e21b57a35ff297dc8c4a15563ea/QuiltView/QuiltView.py . Use it to cross-check behaviour and to find features or edge cases (couples without children, multiple marriages, missing data) the original handles. **Check its license header first** (Gramps is, as far as I know, GPL-licensed, which would make copied or ported code GPL-bound). Unless I approve at Gate A, treat it as a *behavioural reference only*: write specifications and tests from what it does, do not copy or translate its code.
4. Anything else you find (other quilt or matrix-style genealogy views): same rule, license first.

**Provenance rule:** keep `docs/sources.md` listing every external source consulted, its license, what was taken from it (idea, behaviour, algorithm, code) and how it was used. No code or data under a license that conflicts with the project's license enters the repository without my explicit approval. I will choose the project's license at Gate A, after seeing what the original allows.
