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
