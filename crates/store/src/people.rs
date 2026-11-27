//! People, their names, and search.

use crate::access::AccessContext;
use crate::dates::FuzzyDate;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, now_iso};
use crate::model::*;
use crate::names::{compose_full_name, search_key};
use crate::store::*;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Default)]
pub struct NamePatch {
    pub name_type: Option<NameType>,
    pub given: Field<String>,
    pub family: Field<String>,
    pub full_name: Option<String>,
    pub script: Field<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Matched against every name of a person, ignoring case, accents, spaces and Arabic/Persian letter variants.
    pub text: String,
    pub status: Option<Status>,
    /// 0 = default (50). Capped at 1000.
    pub limit: usize,
}

const PERSON_COLS: &str = "p.id, p.sex, p.is_living, p.birth_date, p.birth_qualifier, bp.id, bp.name, p.death_date, p.death_qualifier, dp.id, dp.name, p.note, p.status, p.proposal_id, p.created_by, p.agent_note, p.created_at, p.updated_at";
const PERSON_FROM: &str = "FROM person p LEFT JOIN place bp ON bp.id = p.birth_place_id LEFT JOIN place dp ON dp.id = p.death_place_id";
const DISPLAY_NAME: &str = "COALESCE((SELECT full_name FROM person_name WHERE person_id = p.id AND deleted_at IS NULL ORDER BY is_primary DESC, sort_order, id LIMIT 1), '(unnamed)')";

fn place_ref(id: Option<String>, name: Option<String>) -> Option<PlaceRef> {
    match (id, name) {
        (Some(id), Some(name)) => Some(PlaceRef { id, name }),
        _ => None,
    }
}

fn parse_enum<T>(s: &str, f: fn(&str) -> Result<T>) -> Result<T> {
    f(s).map_err(|_| StoreError::Corrupt(format!("stored value '{s}'")))
}

pub(crate) fn load_names(conn: &Connection, person_id: &str) -> Result<Vec<PersonName>> {
    let mut stmt = conn
        .prepare("SELECT id, type, given, family, full_name, script, is_primary, sort_order FROM person_name WHERE person_id = ?1 AND deleted_at IS NULL ORDER BY is_primary DESC, sort_order, id")?;
    let rows = stmt.query_map([person_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, i64>(6)?, r.get(7)?)))?;
    rows.map(|row| {
        let (id, t, given, family, full_name, script, primary, sort_order) = row?;
        Ok(PersonName { id, name_type: parse_enum(&t, NameType::parse)?, given, family, full_name, script, is_primary: primary == 1, sort_order })
    })
    .collect()
}

pub(crate) fn load_person(conn: &Connection, ctx: &AccessContext, id: &str) -> Result<Person> {
    let sql = format!("SELECT {PERSON_COLS}, {DISPLAY_NAME} {PERSON_FROM} WHERE p.id = ?1 AND p.deleted_at IS NULL AND {}", ctx.row_filter("p"));
    let row = conn
        .query_row(&sql, [id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                place_ref(r.get(5)?, r.get(6)?),
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                place_ref(r.get(9)?, r.get(10)?),
                r.get::<_, Option<String>>(11)?,
                r.get::<_, String>(12)?,
                r.get::<_, Option<String>>(13)?,
                r.get::<_, String>(14)?,
                r.get::<_, Option<String>>(15)?,
                r.get::<_, String>(16)?,
                r.get::<_, String>(17)?,
                r.get::<_, String>(18)?,
            ))
        })
        .optional()?;
    let (pid, sex, living, bd, bq, bplace, dd, dq, dplace, note, status, proposal_id, created_by, agent_note, created_at, updated_at, display_name) =
        row.ok_or_else(|| StoreError::NotFound { kind: "person", id: id.to_string() })?;
    Ok(Person {
        names: load_names(conn, &pid)?,
        id: pid,
        display_name,
        sex: parse_enum(&sex, Sex::parse)?,
        is_living: living == 1,
        birth: decode_fuzzy(bd, bq)?,
        birth_place: bplace,
        death: decode_fuzzy(dd, dq)?,
        death_place: dplace,
        note,
        status: parse_enum(&status, Status::parse)?,
        proposal_id,
        created_by,
        agent_note,
        created_at,
        updated_at,
    })
}

fn living_flag(explicit: Option<bool>, death: &Option<FuzzyDate>) -> Result<bool> {
    match (explicit, death) {
        (Some(true), Some(_)) => Err(StoreError::Conflict("a person with a death date cannot be marked as living".into())),
        (Some(l), _) => Ok(l),
        (None, Some(_)) => Ok(false),
        (None, None) => Ok(true),
    }
}

fn check_origin(origin: &Origin) -> Result<()> {
    if origin.status == Status::Pending && origin.proposal_id.is_none() {
        return Err(StoreError::invalid("status", "pending data must belong to a proposal"));
    }
    check_len("agent_note", origin.agent_note.as_deref().unwrap_or(""), 1000)
}

struct PreparedName {
    name_type: NameType,
    given: Option<String>,
    family: Option<String>,
    full_name: String,
    script: Option<String>,
}

fn prepare_name(n: &NewName) -> Result<PreparedName> {
    let given = clean(n.given.as_deref());
    let family = clean(n.family.as_deref());
    let full_name = clean(n.full_name.as_deref()).unwrap_or_else(|| compose_full_name(given.as_deref(), family.as_deref()));
    if full_name.is_empty() {
        return Err(StoreError::invalid("name", "a name is required (full name, or given and family name)"));
    }
    for (f, v) in [("full_name", Some(full_name.as_str())), ("given", given.as_deref()), ("family", family.as_deref())] {
        check_len(f, v.unwrap_or(""), MAX_NAME)?;
    }
    Ok(PreparedName { name_type: n.name_type.unwrap_or(NameType::Birth), given, family, full_name, script: clean(n.script.as_deref()) })
}

fn insert_name(conn: &Connection, person_id: &str, n: &PreparedName, primary: bool, sort_order: i64, by: &str) -> Result<String> {
    let id = new_id();
    conn.execute(
        "INSERT INTO person_name(id, person_id, type, given, family, full_name, search_key, script, is_primary, sort_order) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![id, person_id, n.name_type.as_str(), n.given, n.family, n.full_name, search_key(&n.full_name), n.script, primary as i64, sort_order],
    )?;
    let after = snapshot(conn, "person_name", &id)?;
    write_revision(conn, "person_name", &id, None, after.as_ref(), by, None)?;
    Ok(id)
}

fn touch_person(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("UPDATE person SET updated_at = ?2 WHERE id = ?1", params![id, now_iso()])?;
    Ok(())
}

impl Store {
    pub fn create_person(&mut self, ctx: &AccessContext, new: NewPerson, origin: &Origin) -> Result<Person> {
        check_origin(origin)?;
        if new.names.is_empty() {
            return Err(StoreError::invalid("name", "a person needs at least one name"));
        }
        let names = new.names.iter().map(prepare_name).collect::<Result<Vec<_>>>()?;
        let is_living = living_flag(new.is_living, &new.death)?;
        let note = clean(new.note.as_deref());
        check_len("note", note.as_deref().unwrap_or(""), MAX_NOTE)?;

        let id = new_id();
        let tx = self.begin()?;
        let by = origin.created_by.as_str();
        let bplace = clean(new.birth_place.as_deref()).map(|p| resolve_place(&tx, &p, by)).transpose()?;
        let dplace = clean(new.death_place.as_deref()).map(|p| resolve_place(&tx, &p, by)).transpose()?;
        let t = now_iso();
        tx.execute(
            "INSERT INTO person(id, sex, is_living, birth_date, birth_qualifier, birth_place_id, death_date, death_qualifier, death_place_id, note, status, proposal_id, created_by, agent_note, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)",
            params![
                id,
                new.sex.as_str(),
                is_living as i64,
                new.birth.map(|d| d.date.to_string()),
                new.birth.map(|d| d.qualifier.as_str()),
                bplace,
                new.death.map(|d| d.date.to_string()),
                new.death.map(|d| d.qualifier.as_str()),
                dplace,
                note,
                origin.status.as_str(),
                origin.proposal_id,
                origin.created_by,
                clean(origin.agent_note.as_deref()),
                t
            ],
        )?;
        for (i, n) in names.iter().enumerate() {
            insert_name(&tx, &id, n, i == 0, i as i64, by)?;
        }
        let after = snapshot(&tx, "person", &id)?;
        write_revision(&tx, "person", &id, None, after.as_ref(), by, origin.proposal_id.as_deref())?;
        let person = load_person(&tx, ctx, &id)?;
        tx.commit()?;
        Ok(person)
    }

    pub fn get_person(&self, ctx: &AccessContext, id: &str) -> Result<Person> {
        load_person(&self.conn, ctx, id)
    }

    /// Apply a patch. An edit that changes nothing writes no revision and leaves `updated_at` alone.
    pub fn update_person(&mut self, ctx: &AccessContext, id: &str, patch: PersonPatch, by: &str) -> Result<Person> {
        let tx = self.begin()?;
        let cur = load_person(&tx, ctx, id)?;
        let before = snapshot(&tx, "person", id)?;

        let birth = match patch.birth {
            Field::Unchanged => cur.birth,
            Field::Set(d) => Some(d),
            Field::Clear => None,
        };
        let death = match patch.death {
            Field::Unchanged => cur.death,
            Field::Set(d) => Some(d),
            Field::Clear => None,
        };
        // Setting a death date implies "not living" unless the caller said something explicit.
        let explicit_living = patch.is_living.or(if matches!(patch.death, Field::Set(_)) { None } else { Some(cur.is_living) });
        let is_living = living_flag(explicit_living, &death)?;
        let place = |f: &Field<String>, cur: &Option<PlaceRef>| -> Result<Option<String>> {
            Ok(match f {
                Field::Unchanged => cur.as_ref().map(|p| p.id.clone()),
                Field::Clear => None,
                Field::Set(name) => Some(resolve_place(&tx, name, by)?),
            })
        };
        let bplace = place(&patch.birth_place, &cur.birth_place)?;
        let dplace = place(&patch.death_place, &cur.death_place)?;
        let note = match &patch.note {
            Field::Unchanged => cur.note.clone(),
            Field::Clear => None,
            Field::Set(n) => clean(Some(n)),
        };
        check_len("note", note.as_deref().unwrap_or(""), MAX_NOTE)?;

        tx.execute(
            "UPDATE person SET sex=?2, is_living=?3, birth_date=?4, birth_qualifier=?5, birth_place_id=?6, death_date=?7, death_qualifier=?8, death_place_id=?9, note=?10, updated_at=?11 WHERE id=?1",
            params![
                id,
                patch.sex.unwrap_or(cur.sex).as_str(),
                is_living as i64,
                birth.map(|d| d.date.to_string()),
                birth.map(|d| d.qualifier.as_str()),
                bplace,
                death.map(|d| d.date.to_string()),
                death.map(|d| d.qualifier.as_str()),
                dplace,
                note,
                now_iso()
            ],
        )?;
        let after = snapshot(&tx, "person", id)?;
        match (&before, &after) {
            (Some(b), Some(a)) if same_content(b, a) => return Ok(cur), // nothing changed: roll back
            _ => {}
        }
        write_revision(&tx, "person", id, before.as_ref(), after.as_ref(), by, None)?;
        let person = load_person(&tx, ctx, id)?;
        tx.commit()?;
        Ok(person)
    }

    /// Soft-delete a person and every link they are in. Returns how many links went with them.
    pub fn soft_delete_person(&mut self, ctx: &AccessContext, id: &str, by: &str) -> Result<usize> {
        let tx = self.begin()?;
        load_person(&tx, ctx, id)?; // exists and visible
        let t = now_iso();
        let link_ids: Vec<String> = {
            let mut s = tx.prepare("SELECT id FROM relationship WHERE (person_a = ?1 OR person_b = ?1) AND deleted_at IS NULL ORDER BY id")?;
            let v = s.query_map([id], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            v
        };
        for lid in &link_ids {
            let before = snapshot(&tx, "relationship", lid)?;
            tx.execute("UPDATE relationship SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1", params![lid, t])?;
            let after = snapshot(&tx, "relationship", lid)?;
            write_revision(&tx, "relationship", lid, before.as_ref(), after.as_ref(), by, None)?;
        }
        let before = snapshot(&tx, "person", id)?;
        tx.execute("UPDATE person SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1", params![id, t])?;
        let after = snapshot(&tx, "person", id)?;
        write_revision(&tx, "person", id, before.as_ref(), after.as_ref(), by, None)?;
        tx.commit()?;
        Ok(link_ids.len())
    }

    // ───────────────────────────── names ─────────────────────────────

    /// Add a name. It becomes the primary name if asked to, or if the person has none.
    pub fn add_name(&mut self, ctx: &AccessContext, person_id: &str, name: NewName, make_primary: bool, by: &str) -> Result<PersonName> {
        let prepared = prepare_name(&name)?;
        let tx = self.begin()?;
        load_person(&tx, ctx, person_id)?;
        let has_primary: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM person_name WHERE person_id = ?1 AND is_primary = 1 AND deleted_at IS NULL)", [person_id], |r| r.get(0))?;
        let primary = make_primary || !has_primary;
        if primary && has_primary {
            demote_primary(&tx, person_id, by)?;
        }
        let next: i64 = tx.query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM person_name WHERE person_id = ?1", [person_id], |r| r.get(0))?;
        let id = insert_name(&tx, person_id, &prepared, primary, next, by)?;
        touch_person(&tx, person_id)?;
        let out = load_names(&tx, person_id)?.into_iter().find(|n| n.id == id).ok_or_else(|| StoreError::Corrupt("new name vanished".into()))?;
        tx.commit()?;
        Ok(out)
    }

    pub fn update_name(&mut self, ctx: &AccessContext, name_id: &str, patch: NamePatch, by: &str) -> Result<PersonName> {
        let tx = self.begin()?;
        let (person_id, cur) = name_owner(&tx, name_id)?;
        load_person(&tx, ctx, &person_id)?;
        let before = snapshot(&tx, "person_name", name_id)?;
        let apply = |f: &Field<String>, cur: &Option<String>| match f {
            Field::Unchanged => cur.clone(),
            Field::Clear => None,
            Field::Set(v) => clean(Some(v)),
        };
        let given = apply(&patch.given, &cur.given);
        let family = apply(&patch.family, &cur.family);
        let parts_changed = !matches!(patch.given, Field::Unchanged) || !matches!(patch.family, Field::Unchanged);
        let full_name = match (&patch.full_name, parts_changed) {
            (Some(f), _) => clean(Some(f)).unwrap_or_default(),
            (None, true) => compose_full_name(given.as_deref(), family.as_deref()),
            (None, false) => cur.full_name.clone(),
        };
        let prepared = prepare_name(&NewName { name_type: Some(patch.name_type.unwrap_or(cur.name_type)), given, family, full_name: Some(full_name), script: apply(&patch.script, &cur.script) })?;
        tx.execute(
            "UPDATE person_name SET type=?2, given=?3, family=?4, full_name=?5, search_key=?6, script=?7 WHERE id=?1",
            params![name_id, prepared.name_type.as_str(), prepared.given, prepared.family, prepared.full_name, search_key(&prepared.full_name), prepared.script],
        )?;
        let after = snapshot(&tx, "person_name", name_id)?;
        if before != after {
            write_revision(&tx, "person_name", name_id, before.as_ref(), after.as_ref(), by, None)?;
            touch_person(&tx, &person_id)?;
        }
        let out = load_names(&tx, &person_id)?.into_iter().find(|n| n.id == name_id).ok_or_else(|| StoreError::Corrupt("name vanished".into()))?;
        tx.commit()?;
        Ok(out)
    }

    pub fn set_primary_name(&mut self, ctx: &AccessContext, name_id: &str, by: &str) -> Result<()> {
        let tx = self.begin()?;
        let (person_id, cur) = name_owner(&tx, name_id)?;
        load_person(&tx, ctx, &person_id)?;
        if cur.is_primary {
            return Ok(());
        }
        demote_primary(&tx, &person_id, by)?;
        let before = snapshot(&tx, "person_name", name_id)?;
        tx.execute("UPDATE person_name SET is_primary = 1 WHERE id = ?1", [name_id])?;
        let after = snapshot(&tx, "person_name", name_id)?;
        write_revision(&tx, "person_name", name_id, before.as_ref(), after.as_ref(), by, None)?;
        touch_person(&tx, &person_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Soft-delete a name. The last name of a person cannot be removed; if the primary goes, the next one is promoted.
    pub fn remove_name(&mut self, ctx: &AccessContext, name_id: &str, by: &str) -> Result<()> {
        let tx = self.begin()?;
        let (person_id, cur) = name_owner(&tx, name_id)?;
        load_person(&tx, ctx, &person_id)?;
        let live: i64 = tx.query_row("SELECT count(*) FROM person_name WHERE person_id = ?1 AND deleted_at IS NULL", [&person_id], |r| r.get(0))?;
        if live <= 1 {
            return Err(StoreError::Conflict("a person must keep at least one name".into()));
        }
        let before = snapshot(&tx, "person_name", name_id)?;
        tx.execute("UPDATE person_name SET deleted_at = ?2, is_primary = 0 WHERE id = ?1", params![name_id, now_iso()])?;
        let after = snapshot(&tx, "person_name", name_id)?;
        write_revision(&tx, "person_name", name_id, before.as_ref(), after.as_ref(), by, None)?;
        if cur.is_primary {
            let next: String = tx.query_row("SELECT id FROM person_name WHERE person_id = ?1 AND deleted_at IS NULL ORDER BY sort_order, id LIMIT 1", [&person_id], |r| r.get(0))?;
            let b = snapshot(&tx, "person_name", &next)?;
            tx.execute("UPDATE person_name SET is_primary = 1 WHERE id = ?1", [&next])?;
            let a = snapshot(&tx, "person_name", &next)?;
            write_revision(&tx, "person_name", &next, b.as_ref(), a.as_ref(), by, None)?;
        }
        touch_person(&tx, &person_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Every live person visible to `ctx`, ordered by id (creation order). For exports and tools.
    pub fn all_people(&self, ctx: &AccessContext, include_pending: bool) -> Result<Vec<Person>> {
        let sql = format!("SELECT p.id FROM person p WHERE p.deleted_at IS NULL AND p.merged_into IS NULL AND (p.status = 'verified' OR ?1 = 1) AND {} ORDER BY p.id", ctx.row_filter("p"));
        let ids: Vec<String> = {
            let mut stmt = self.conn.prepare(&sql)?;
            let v = stmt.query_map([include_pending as i64], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            v
        };
        ids.iter().map(|id| load_person(&self.conn, ctx, id)).collect()
    }

    /// One name entry with the person it belongs to.
    pub fn find_name(&self, ctx: &AccessContext, name_id: &str) -> Result<(String, PersonName)> {
        let person_id: Option<String> = self.conn.query_row("SELECT person_id FROM person_name WHERE id = ?1 AND deleted_at IS NULL", [name_id], |r| r.get(0)).optional()?;
        let person_id = person_id.ok_or_else(|| StoreError::NotFound { kind: "name", id: name_id.to_string() })?;
        load_person(&self.conn, ctx, &person_id)?; // visible and not deleted
        let name = load_names(&self.conn, &person_id)?.into_iter().find(|n| n.id == name_id).ok_or_else(|| StoreError::NotFound { kind: "name", id: name_id.to_string() })?;
        Ok((person_id, name))
    }

    // ───────────────────────────── search ─────────────────────────────

    /// Ids of every person who has a name containing `key` (an already normalised search key), looking at ALL of a
    /// person's names exactly as [`Store::search_people_page`] does. For filtering a list the same way search matches.
    pub fn person_ids_with_name(&self, key: &str) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT person_id FROM person_name WHERE deleted_at IS NULL AND instr(search_key, ?1) > 0")?;
        let ids = stmt.query_map([key], |r| r.get::<_, String>(0))?.collect::<std::result::Result<_, _>>()?;
        Ok(ids)
    }

    pub fn search_people(&self, ctx: &AccessContext, q: &SearchQuery) -> Result<Vec<PersonSummary>> {
        let limit = if q.limit == 0 { 50 } else { q.limit.min(1000) };
        Ok(self.search_people_page(ctx, &SearchQuery { limit, ..q.clone() }, None)?.into_iter().map(|(p, _)| p).collect())
    }

    /// Search in a stable order (the position where the text first matches a name, then id) and continue after a
    /// row: `after` is the `(position, id)` returned with the last row of the previous page, so a person added or
    /// changed between pages can never make one repeat or go missing. `q.limit` is used as given (at most 1001, so
    /// a caller can ask for one more than it shows to know whether a next page exists).
    pub fn search_people_page(&self, ctx: &AccessContext, q: &SearchQuery, after: Option<(i64, &str)>) -> Result<Vec<(PersonSummary, i64)>> {
        let limit = q.limit.clamp(1, 1001) as i64;
        let key = search_key(&q.text);
        let (after_pos, after_id) = match after {
            Some((pos, id)) => (Some(pos), Some(id.to_string())),
            None => (None, None),
        };
        let sql = format!(
            "SELECT p.id, {DISPLAY_NAME}, p.sex, p.is_living, p.birth_date, p.birth_qualifier, p.death_date, p.death_qualifier, p.status, MIN(instr(n.search_key, ?1)) AS pos
             FROM person p JOIN person_name n ON n.person_id = p.id AND n.deleted_at IS NULL
             WHERE p.deleted_at IS NULL AND p.merged_into IS NULL AND instr(n.search_key, ?1) > 0 AND (?2 IS NULL OR p.status = ?2) AND {}
             GROUP BY p.id
             HAVING (?4 IS NULL OR MIN(instr(n.search_key, ?1)) > ?4 OR (MIN(instr(n.search_key, ?1)) = ?4 AND p.id > ?5))
             ORDER BY pos, p.id LIMIT ?3",
            ctx.row_filter("p")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let status = q.status.map(|s| s.as_str());
        let rows = stmt.query_map(params![key, status, limit, after_pos, after_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (id, display_name, sex, living, bd, bq, dd, dq, status, pos) = row?;
            Ok((
                PersonSummary {
                    id,
                    display_name,
                    sex: parse_enum(&sex, Sex::parse)?,
                    is_living: living == 1,
                    birth: decode_fuzzy(bd, bq)?,
                    death: decode_fuzzy(dd, dq)?,
                    status: parse_enum(&status, Status::parse)?,
                },
                pos,
            ))
        })
        .collect()
    }
}

fn demote_primary(conn: &Connection, person_id: &str, by: &str) -> Result<()> {
    let ids: Vec<String> = {
        let mut s = conn.prepare("SELECT id FROM person_name WHERE person_id = ?1 AND is_primary = 1 AND deleted_at IS NULL")?;
        let v = s.query_map([person_id], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
        v
    };
    for id in ids {
        let b = snapshot(conn, "person_name", &id)?;
        conn.execute("UPDATE person_name SET is_primary = 0 WHERE id = ?1", [&id])?;
        let a = snapshot(conn, "person_name", &id)?;
        write_revision(conn, "person_name", &id, b.as_ref(), a.as_ref(), by, None)?;
    }
    Ok(())
}

fn name_owner(conn: &Connection, name_id: &str) -> Result<(String, PersonName)> {
    let owner: Option<String> = conn.query_row("SELECT person_id FROM person_name WHERE id = ?1 AND deleted_at IS NULL", [name_id], |r| r.get(0)).optional()?;
    let person_id = owner.ok_or_else(|| StoreError::NotFound { kind: "name", id: name_id.to_string() })?;
    let n = load_names(conn, &person_id)?.into_iter().find(|n| n.id == name_id).ok_or_else(|| StoreError::NotFound { kind: "name", id: name_id.to_string() })?;
    Ok((person_id, n))
}
