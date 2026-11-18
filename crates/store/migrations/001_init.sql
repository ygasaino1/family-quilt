-- Migration 1: every table of the data model (section 5 of the spec).
--
-- Conventions
--   * ids are UUIDv7 text; timestamps are UTC ISO-8601 text.
--   * soft deletes only (`deleted_at`); nothing is ever DELETEd by the app.
--   * `vault_id` is on every table, NULL = public. v1 never sets it (section 16).
--   * Dates are `YYYY`, `YYYY-MM` or `YYYY-MM-DD` plus a qualifier.

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT
);

-- One agent request, reviewed as a whole by the human.
CREATE TABLE proposal (
  id          TEXT PRIMARY KEY,
  summary     TEXT,
  source_text TEXT,                       -- the user's own words, shown next to what the agent understood
  created_by  TEXT NOT NULL,              -- 'human' | 'agent:<name>'
  status      TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected')),
  vault_id    TEXT DEFAULT NULL,
  created_at  TEXT NOT NULL,
  decided_at  TEXT
);
CREATE INDEX idx_proposal_status ON proposal(status);

CREATE TABLE place (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  name_key   TEXT NOT NULL,               -- names::search_key(name); find-or-create matches on it
  parent_id  TEXT REFERENCES place(id),
  note       TEXT,
  vault_id   TEXT DEFAULT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT
);
CREATE UNIQUE INDEX idx_place_key ON place(name_key, COALESCE(parent_id, '')) WHERE deleted_at IS NULL;

CREATE TABLE person (
  id              TEXT PRIMARY KEY,
  sex             TEXT NOT NULL DEFAULT 'U' CHECK (sex IN ('M','F','X','U')),
  is_living       INTEGER NOT NULL DEFAULT 1 CHECK (is_living IN (0,1)),
  birth_date      TEXT,
  birth_qualifier TEXT CHECK (birth_qualifier IN ('exact','about','before','after')),
  birth_place_id  TEXT REFERENCES place(id),
  death_date      TEXT,
  death_qualifier TEXT CHECK (death_qualifier IN ('exact','about','before','after')),
  death_place_id  TEXT REFERENCES place(id),
  note            TEXT,                   -- facts worth keeping that have no field (section 19 allows agents to propose edits to it)
  status          TEXT NOT NULL DEFAULT 'verified' CHECK (status IN ('verified','pending')),
  vault_id        TEXT DEFAULT NULL,
  proposal_id     TEXT REFERENCES proposal(id),
  created_by      TEXT NOT NULL DEFAULT 'human',
  agent_note      TEXT,
  merged_into     TEXT REFERENCES person(id),
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  deleted_at      TEXT,
  CHECK ((birth_date IS NULL) = (birth_qualifier IS NULL)),
  CHECK ((death_date IS NULL) = (death_qualifier IS NULL)),
  CHECK (birth_date IS NULL OR (length(birth_date) IN (4,7,10) AND birth_date NOT GLOB '*[^0-9-]*')),
  CHECK (death_date IS NULL OR (length(death_date) IN (4,7,10) AND death_date NOT GLOB '*[^0-9-]*')),
  CHECK (death_date IS NULL OR is_living = 0)
);
CREATE INDEX idx_person_status   ON person(status) WHERE deleted_at IS NULL;
CREATE INDEX idx_person_proposal ON person(proposal_id);

-- Many names per person, any script.
CREATE TABLE person_name (
  id         TEXT PRIMARY KEY,
  person_id  TEXT NOT NULL REFERENCES person(id),
  type       TEXT NOT NULL DEFAULT 'birth' CHECK (type IN ('birth','married','alias','religious','other')),
  given      TEXT,
  family     TEXT,
  full_name  TEXT NOT NULL CHECK (length(full_name) > 0),
  search_key TEXT NOT NULL,               -- names::search_key(full_name), for search and duplicate checks
  script     TEXT,                        -- ISO 15924, e.g. 'Latn', 'Arab'
  is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0,1)),
  sort_order INTEGER NOT NULL DEFAULT 0,
  vault_id   TEXT DEFAULT NULL,
  deleted_at TEXT
);
CREATE INDEX idx_name_person ON person_name(person_id);
CREATE INDEX idx_name_key    ON person_name(search_key);
CREATE UNIQUE INDEX idx_name_one_primary ON person_name(person_id) WHERE is_primary = 1 AND deleted_at IS NULL;

CREATE TABLE relationship (
  id          TEXT PRIMARY KEY,
  type        TEXT NOT NULL CHECK (type IN ('parent_child','partnership')),
  person_a    TEXT NOT NULL REFERENCES person(id),   -- parent_child: the parent. partnership: the smaller id
  person_b    TEXT NOT NULL REFERENCES person(id),   -- parent_child: the child
  kind        TEXT,                                  -- parent_child only (see CHECK)
  subtype     TEXT,                                  -- partnership only (see CHECK)
  start_date  TEXT,
  end_date    TEXT,
  place_id    TEXT REFERENCES place(id),
  note        TEXT,
  status      TEXT NOT NULL DEFAULT 'verified' CHECK (status IN ('verified','pending')),
  vault_id    TEXT DEFAULT NULL,
  proposal_id TEXT REFERENCES proposal(id),
  created_by  TEXT NOT NULL DEFAULT 'human',
  agent_note  TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,                         -- needed by section 19 (stale-change detection)
  deleted_at  TEXT,
  CHECK (person_a <> person_b),
  CHECK (
    (type = 'parent_child' AND subtype IS NULL
       AND kind IN ('birth','adopted','foster','step','guardian','sealing','unknown'))
    OR
    (type = 'partnership' AND kind IS NULL AND person_a < person_b
       AND subtype IN ('married','unmarried','divorced','ended','unknown'))
  ),
  CHECK (start_date IS NULL OR (length(start_date) IN (4,7,10) AND start_date NOT GLOB '*[^0-9-]*')),
  CHECK (end_date   IS NULL OR (length(end_date)   IN (4,7,10) AND end_date   NOT GLOB '*[^0-9-]*'))
);
-- UNIQUE(type, person_a, person_b) among live rows, so a deleted link can be entered again.
CREATE UNIQUE INDEX idx_rel_unique ON relationship(type, person_a, person_b) WHERE deleted_at IS NULL;
CREATE INDEX idx_rel_a ON relationship(person_a, type);
CREATE INDEX idx_rel_b ON relationship(person_b, type);
CREATE INDEX idx_rel_proposal ON relationship(proposal_id);

CREATE TABLE source (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  detail     TEXT,
  vault_id   TEXT DEFAULT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT
);

CREATE TABLE event (
  id          TEXT PRIMARY KEY,
  person_id   TEXT NOT NULL REFERENCES person(id),
  type        TEXT NOT NULL,
  date        TEXT,
  qualifier   TEXT CHECK (qualifier IN ('exact','about','before','after')),
  place_id    TEXT REFERENCES place(id),
  description TEXT,
  vault_id    TEXT DEFAULT NULL,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  deleted_at  TEXT
);
CREATE INDEX idx_event_person ON event(person_id);

CREATE TABLE citation (
  id          TEXT PRIMARY KEY,
  source_id   TEXT NOT NULL REFERENCES source(id),
  target_type TEXT NOT NULL CHECK (target_type IN ('person','relationship','event')),
  target_id   TEXT NOT NULL,
  note        TEXT,
  vault_id    TEXT DEFAULT NULL,
  created_at  TEXT NOT NULL,
  deleted_at  TEXT
);
CREATE INDEX idx_citation_target ON citation(target_type, target_id);

-- Section 19: an agent's proposed edit of an existing record. Logic arrives in Phase 4;
-- the table exists now so migration 1 holds the whole model.
CREATE TABLE change (
  id              TEXT PRIMARY KEY,
  proposal_id     TEXT NOT NULL REFERENCES proposal(id),
  target_type     TEXT NOT NULL CHECK (target_type IN ('person','person_name','relationship')),
  target_id       TEXT NOT NULL,
  op              TEXT NOT NULL CHECK (op IN ('update','remove')),
  patch           TEXT,                   -- JSON {field: {from, to}}
  base_updated_at TEXT,
  agent_note      TEXT,
  status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','accepted','rejected','applied','conflicting')),
  vault_id        TEXT DEFAULT NULL
);
CREATE INDEX idx_change_proposal ON change(proposal_id);
CREATE INDEX idx_change_target   ON change(target_type, target_id);

-- History of every applied change (human edits and approved proposals alike).
CREATE TABLE revision (
  id          TEXT PRIMARY KEY,
  target_type TEXT NOT NULL CHECK (target_type IN ('person','person_name','relationship','place')),
  target_id   TEXT NOT NULL,
  before      TEXT,                       -- JSON snapshot of the row, NULL for a creation
  after       TEXT,                       -- JSON snapshot of the row, NULL never (deletes are soft: deleted_at is set)
  by          TEXT NOT NULL,              -- 'human' | 'agent:<name>'
  at          TEXT NOT NULL,
  proposal_id TEXT REFERENCES proposal(id),
  vault_id    TEXT DEFAULT NULL
);
CREATE INDEX idx_revision_target ON revision(target_type, target_id, at);
