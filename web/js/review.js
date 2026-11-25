// The review panel, docked beside the quilt: what an AI assistant proposed, in your words and theirs.
import { t, fmtAgo, fmtDate } from './i18n.js';
import { pref } from './prefs.js';
import { api } from './api.js';
import { toInput, fromInput, DATE_FIELDS } from './dates.js';
import { h, toast, sexWord, sexLetter, fill, icon } from './dom.js';

let app;
let selected = null;
const panel = () => document.getElementById('panel');
const root = () => document.getElementById('review-root');

export function init(a) {
  app = a;
}

export function hide() {
  panel().hidden = true;
}

export function onDataChanged() {
  if (!panel().hidden) show(selected);
}

const val = (v) => (v === null || v === undefined || v === '' ? h('i', { class: 'muted' }, t('review.empty')) : typeof v === 'object' ? JSON.stringify(v) : String(v));
const agentName = (by) => String(by || '').replace(/^agent:/, '') || t('review.someone');
const born = (q) => (q.birth ? toInput(String(q.birth.date ?? q.birth)) : '');
/** A value to show: dates read in the chosen calendar, everything else as it is. */
const dval = (field, v) => (DATE_FIELDS.includes(field) && v && pref('calendar') === 'jalali' ? h('span', {}, fmtDate(String(v), 'exact')) : val(v));
/** The text to store for a typed value: dates typed in another calendar are converted by the server. */
async function stored(field, text) {
  if (!DATE_FIELDS.includes(field) || !text) return text;
  const r = await fromInput(text);
  if (r.approximate && r.note) toast(r.note);
  return r.stored;
}
const WARN_CODES = ['parent_too_young', 'parent_too_old', 'parent_died_before_birth', 'death_before_birth', 'possible_duplicate', 'likely_duplicate', 'link_kind_impact', 'person_removal_impact'];

export async function show(id) {
  selected = id || selected;
  panel().hidden = false;
  let list;
  try {
    list = await api('/proposals?status=pending');
  } catch (e) {
    return fill(root(), h('p', { class: 'conflict' }, e.message));
  }
  const props = list.proposals;
  if (!props.length) {
    selected = null;
    return fill(root(), h('p', { class: 'eyebrow', id: 'panel-title' }, t('review.title')), h('div', { class: 'prop' }, t('review.none')));
  }
  if (!props.some((p) => p.id === selected)) selected = props[0].id;
  const detail = h('div', { id: 'review-detail', class: 'prop' }, t('common.loading'));
  fill(root(), h('p', { class: 'eyebrow', id: 'panel-title' }, t('review.title')), detail,
    props.filter((p) => p.id !== selected).map((p) =>
      h('button', { class: 'prop collapsed', onclick: () => show(p.id) },
        h('span', { class: 'meta' }, h('span', {}, t('review.from', { agent: agentName(p.created_by) })), h('span', {}, fmtAgo(p.created_at))),
        h('b', {}, p.summary || t('review.no_summary')), h('span', { class: 'dot', 'aria-hidden': 'true' }))));
  await renderDetail(detail, selected);
}

// ---- correcting a value before approving ----
const QUALS = ['exact', 'about', 'before', 'after'];
const ENUMS = {
  sex: { values: ['M', 'F', 'X', 'U'], label: (v) => t('sex.' + sexWord(v)) },
  birth_qualifier: { values: QUALS, label: (v) => t('q.' + v) },
  death_qualifier: { values: QUALS, label: (v) => t('q.' + v) },
  kind: { values: ['birth', 'adopted', 'foster', 'step', 'guardian', 'sealing', 'unknown'], label: (v) => t('kind.' + v) },
  subtype: { values: ['married', 'unmarried', 'divorced', 'ended', 'unknown'], label: (v) => t('state.' + v + '.long') },
  is_living: { values: ['true', 'false'], label: (v) => (v === 'true' ? t('common.yes') : t('common.no')) },
};

/** An input for one field: a list for the fixed choices, a text box otherwise. `get()` gives the text typed or chosen. */
function fieldInput(field, value, id) {
  const e = ENUMS[field];
  const date = DATE_FIELDS.includes(field);
  const cur = value === null || value === undefined ? '' : date ? toInput(value) : String(value);
  if (e) {
    const sel = h('select', { id, 'aria-label': t('field.' + field) }, ...e.values.map((v) => h('option', { value: v, selected: v === cur }, e.label(v))));
    if (!e.values.includes(cur)) sel.prepend(h('option', { value: '', selected: true }, ''));
    return { el: sel, get: () => sel.value };
  }
  const inp = h('input', { type: 'text', id, value: cur, autocomplete: 'off', 'aria-label': t('field.' + field) });
  return { el: inp, get: () => inp.value.trim(), shown: cur };
}

/** Send corrections for several records; fields are `{kind, target_id, field, value}`. Date and certainty go together. */
async function sendEdits(proposalId, specs, note = '') {
  const groups = new Map();
  for (const s of specs) {
    const k = s.kind + '|' + s.target_id;
    if (!groups.has(k)) groups.set(k, { kind: s.kind, target_id: s.target_id, fields: {} });
    groups.get(k).fields[s.field] = s.value;
  }
  for (const g of groups.values()) {
    for (const [d, q] of [['birth_date', 'birth_qualifier'], ['death_date', 'death_qualifier']]) {
      if (d in g.fields || q in g.fields) {
        const date = d in g.fields ? g.fields[d] : specs.find((s) => s.field === d && s.target_id === g.target_id)?.was;
        if (!date) { g.fields[d] = null; g.fields[q] = null; } else if (!g.fields[q]) g.fields[q] = 'exact';
        if (!(d in g.fields)) g.fields[d] = date;
      }
    }
    for (const f of Object.keys(g.fields)) if (g.fields[f] === '') g.fields[f] = null;
    if ('is_living' in g.fields && g.fields.is_living !== null) g.fields.is_living = g.fields.is_living === 'true';
    await api(`/proposals/${proposalId}/edit`, { method: 'POST', body: { kind: g.kind, target_id: g.target_id, fields: g.fields, note: note.trim() || undefined } });
  }
}

let keep = { id: null, set: new Set() };

/** The server's field-level problems when there are some, else its message. */
const problems = (e) => (e.body?.error?.fields?.length ? e.body.error.fields.map((f) => `${t('field.' + String(f.path).split('.').pop()) || f.path}: ${f.message}`).join('; ') : e.message);

async function renderDetail(box, id) {
  let v;
  try {
    v = await api('/proposals/' + id);
  } catch (e) {
    return fill(box, h('p', { class: 'conflict' }, e.message));
  }
  const p = v.proposal;
  const blocked = v.depends_on.filter((d) => d.status === 'pending');
  const checks = []; // {change, field, box, conflict, over}

  // New people and links can each be left out of the approval (the ✕ on the row); a left-out person takes their links along.
  if (keep.id !== id) keep = { id, set: new Set() };
  const leftOut = keep.set;
  const rows = []; // {el, kind: 'person'|'link', id, a, b}
  const syncOut = () => {
    for (const r of rows) {
      const own = leftOut.has(r.id);
      const viaPerson = r.kind === 'link' && (leftOut.has(r.a) || leftOut.has(r.b));
      r.el.classList.toggle('out', own || viaPerson);
      r.btn.setAttribute('aria-pressed', String(own || viaPerson));
      r.btn.disabled = viaPerson && !own;
      r.btn.title = r.btn.getAttribute('aria-label') + (viaPerson && !own ? ' ' + t('review.left_out_with_person') : '');
    }
    const n = rows.filter((r) => r.el.classList.contains('out')).length;
    const ap = document.getElementById('review-approve');
    if (ap) ap.textContent = n ? t('review.approve_rest', { n }) : t('review.approve');
  };
  const leaveOutBtn = (r) => {
    r.btn = h('button', { class: 'btn tiny', 'aria-pressed': 'false', 'aria-label': t('review.leave_out'), title: t('review.leave_out'), onclick: () => {
      if (leftOut.has(r.id)) leftOut.delete(r.id);
      else leftOut.add(r.id);
      syncOut();
    } }, icon('close', 14));
    rows.push(r);
    return r.btn;
  };
  const items = [];
  const edits = v.edits || [];
  /** The pencil, the "edited" mark and the form that opens under a row. `fields`: [{kind, target_id, field, value}]. */
  const editor = (row, fields, targets) => {
    const mine = edits.filter((e) => targets.includes(e.target_id));
    const inputs = fields.map((f) => ({ ...f, input: fieldInput(f.field, f.value, `e-${f.target_id}-${f.field}`) }));
    const err = h('p', { class: 'conflict', role: 'alert', hidden: true });
    const noteId = `en-${fields[0]?.target_id}`;
    const noteIn = h('input', { id: noteId, type: 'text', maxlength: '300', placeholder: t('review.note_ph') });
    const form = h('div', { class: 'edit-form', hidden: true },
      h('div', { class: 'props' }, ...inputs.flatMap((f) => [h('label', { for: f.input.el.id }, t('field.' + f.field)), f.input.el]), h('label', { for: noteId }, t('review.note')), noteIn), err,
      h('div', { class: 'tools' },
        h('button', { class: 'btn tiny primary', onclick: async () => {
          const dirty = inputs.filter((f) => String(f.input.get()) !== (f.input.shown ?? String(f.value ?? '')));
          if (!dirty.length) { form.hidden = true; return; }
          try {
            const changed = [];
            for (const f of dirty) changed.push({ kind: f.kind, target_id: f.target_id, field: f.field, value: await stored(f.field, f.input.get()), was: f.value });
            await sendEdits(id, changed, noteIn.value);
            await renderDetail(box, id);
          } catch (e) {
            err.textContent = problems(e);
            err.hidden = false;
          }
        } }, t('review.edit_save')),
        mine.length ? h('button', { class: 'btn tiny', onclick: async () => {
          try {
            await sendEdits(id, mine.map((e) => ({ kind: e.kind, target_id: e.target_id, field: e.field, value: e.original, was: e.current })));
            await renderDetail(box, id);
          } catch (e2) {
            err.textContent = problems(e2);
            err.hidden = false;
          }
        } }, t('review.edit_undo')) : null));
    const pencil = h('button', { class: 'btn tiny', 'aria-expanded': 'false', title: t('review.edit'), 'aria-label': t('review.edit'), onclick: () => {
      form.hidden = !form.hidden;
      pencil.setAttribute('aria-expanded', String(!form.hidden));
    } }, icon('edit', 14));
    return { pencil, form, mark: mine.length ? h('span', { class: 'chip', title: mine.map((e) => `${t('field.' + e.field)}: ${e.original ?? ''} → ${e.current ?? ''}`).join('\n') }, t('review.edited')) : null };
  };
  for (const q of v.new_people) {
    const r = { kind: 'person', id: q.id };
    const btn = leaveOutBtn(r);
    const nm = (q.names || []).find((n) => n.is_primary) || (q.names || [])[0];
    const fields = [];
    if (nm) {
      if (nm.given || nm.family) fields.push({ kind: 'person_name', target_id: nm.id, field: 'given', value: nm.given }, { kind: 'person_name', target_id: nm.id, field: 'family', value: nm.family });
      else fields.push({ kind: 'person_name', target_id: nm.id, field: 'full_name', value: nm.full_name });
    }
    const P = (field, value) => fields.push({ kind: 'person', target_id: q.id, field, value });
    P('sex', sexLetter(q.sex));
    P('birth_date', q.birth?.date ?? '');
    P('birth_qualifier', q.birth?.qualifier ?? '');
    P('birth_place', q.birth_place?.name ?? '');
    P('death_date', q.death?.date ?? '');
    P('death_qualifier', q.death?.qualifier ?? '');
    P('death_place', q.death_place?.name ?? '');
    P('note', q.note ?? '');
    const ed = editor(r, fields, [q.id, ...(q.names || []).map((n) => n.id)]);
    r.el = h('div', { class: 'item-row' }, h('span', { class: 'waiting' }, t('review.waiting')), h('span', { class: 'nm' }, q.display_name, born(q) ? ` · ${born(q)} –` : ''), ed.mark,
      h('button', { class: 'link', title: t('review.show_in_tree'), 'aria-label': t('review.show_in_tree'), onclick: () => app.view.q && app.openPerson(q.id) }, icon('tree', 16)), ed.pencil, btn);
    items.push(h('div', { class: 'item' }, r.el, ed.form));
  }
  for (const l of v.new_relationships) {
    const r = { kind: 'link', id: l.id, a: l.person_a, b: l.person_b };
    const btn = leaveOutBtn(r);
    const rel = l.rel || l;
    const fields = rel.rel_type === 'parent_child' ? [{ kind: 'relationship', target_id: rel.id, field: 'kind', value: rel.kind || 'unknown' }] : [{ kind: 'relationship', target_id: rel.id, field: 'subtype', value: rel.subtype || 'unknown' }];
    fields.push({ kind: 'relationship', target_id: rel.id, field: 'start_date', value: rel.start_date ?? '' }, { kind: 'relationship', target_id: rel.id, field: 'end_date', value: rel.end_date ?? '' });
    const ed = editor(r, fields, [rel.id]);
    r.el = h('div', { class: 'item-row' }, h('span', { class: 'waiting' }, t('review.waiting')), h('span', { class: 'nm' },
      l.rel_type === 'parent_child' ? t('review.link_parent', { a: l.a_name, b: l.b_name, kind: t('kind.' + (l.kind || 'unknown')) }) : t('review.link_partner', { a: l.a_name, b: l.b_name, state: t('state.' + (l.subtype || 'unknown') + '.long') })), ed.mark, ed.pencil, btn);
    items.push(h('div', { class: 'item' }, r.el, ed.form));
  }

  const changeBlocks = [];
  for (const c of v.changes) {
    if (c.op === 'remove') {
      const cb = h('input', { type: 'checkbox', id: 'rm-' + c.id });
      checks.push({ change: c, field: 'remove', box: cb });
      changeBlocks.push(h('div', { class: 'warncard' }, icon('warn', 22), h('b', {}, t('review.remove_title', { name: c.target_label })), h('span', {}, t('review.remove_warning')), h('label', {}, cb, ' ', t('review.remove_tick'))));
      continue;
    }
    for (const f of c.fields) {
      const accept = h('input', { type: 'checkbox', id: `f-${c.id}-${f.field}`, checked: !f.conflict, 'aria-label': t('review.accept') });
      const over = f.conflict ? h('input', { type: 'checkbox', id: `o-${c.id}-${f.field}` }) : null;
      checks.push({ change: c, field: f.field, box: accept, over });
      // The new value can be corrected before accepting it; what the assistant proposed stays on record.
      const corrected = edits.find((e) => e.kind === 'change' && e.target_id === c.id && e.field === f.field);
      const toBox = h('span', { class: 'to' }, dval(f.field, f.to), corrected ? h('span', { class: 'chip', title: `${t('review.assistant_said')}: ${corrected.original ?? ''}` }, t('review.edited')) : null);
      const pencil = h('button', { class: 'btn tiny', title: t('review.edit'), 'aria-label': t('review.edit'), onclick: () => {
        const input = fieldInput(f.field, f.to, `ce-${c.id}-${f.field}`);
        const noteIn = h('input', { type: 'text', class: 'note-in', maxlength: '300', placeholder: t('review.note_ph'), 'aria-label': t('review.note') });
        const save = async (value, typed = true) => {
          try {
            await sendEdits(id, [{ kind: 'change', target_id: c.id, field: f.field, value: typed ? await stored(f.field, value) : value, was: f.to }], typed ? noteIn.value : '');
            await renderDetail(box, id);
          } catch (e) {
            toast(problems(e), 'error');
          }
        };
        for (const el of [input.el, noteIn]) el.addEventListener('keydown', (ev) => {
          if (ev.key === 'Enter') save(input.get());
          if (ev.key === 'Escape') { ev.stopPropagation(); renderDetail(box, id); }
        });
        fill(toBox, input.el, noteIn, h('button', { class: 'btn tiny primary', onclick: () => save(input.get()) }, t('review.edit_save')),
          corrected ? h('button', { class: 'btn tiny', onclick: () => save(corrected.original ?? '', false) }, t('review.edit_undo')) : null);
        input.el.focus();
      } }, icon('edit', 14));
      changeBlocks.push(h('div', { class: 'sugg' },
        h('span', { class: 'what' }, c.target_label, ' · ', t('field.' + f.field)),
        h('span', { class: 'from' }, dval(f.field, f.from)), h('span', { class: 'arrow' }, '→'), toBox, pencil,
        h('label', {}, accept, ' ', t('review.accept')),
        f.conflict ? h('div', { class: 'conflict' }, t('review.conflict', { current: String(f.current ?? '') }), h('label', {}, over, ' ', t('review.overwrite'))) : null));
    }
  }

  const warns = v.warnings.map((w) => {
    const existing = Array.isArray(w.params?.existing) ? w.params.existing[0] : null;
    return h('div', { class: 'warncard' }, icon('warn', 22), h('b', {}, WARN_CODES.includes(w.code) ? t('warn.' + w.code) : t('review.warnings')), h('span', {}, w.message),
      existing ? h('button', { class: 'link', onclick: () => app.showScreen('quilt') || app.openPerson(existing) }, t('review.open_existing')) : null);
  });
  const deps = blocked.length ? h('div', { class: 'warncard', id: 'review-blocked' }, icon('warn', 22), h('b', {}, t('review.blocked')), blocked.map((d) => h('button', { class: 'link', onclick: () => show(d.proposal_id) }, d.summary || d.proposal_id))) : null;

  fill(box,
    h('div', { class: 'meta' }, h('span', {}, t('review.from', { agent: agentName(p.created_by) })), h('span', {}, fmtAgo(p.created_at))),
    h('h2', {}, p.summary || t('review.no_summary')),
    h('div', { class: 'told' }, h('p', { class: 'eyebrow' }, t('review.your_words')), p.source_text ? h('div', {}, p.source_text) : h('i', { class: 'muted' }, t('review.no_source'))),
    items.length ? [h('p', { class: 'eyebrow' }, t('review.understood')), h('div', { class: 'items' }, items)] : null,
    changeBlocks.length ? [h('p', { class: 'eyebrow', style: 'margin-top:14px' }, t('review.suggested')), changeBlocks] : null,
    deps, warns,
    h('div', { class: 'actions' },
      h('button', { class: 'btn primary', id: 'review-approve', disabled: blocked.length > 0, onclick: () => doApprove(id, checks, leftOut, v) }, t('review.approve')),
      h('button', { class: 'btn', id: 'review-reject', onclick: () => doReject(id) }, t('review.reject'))));
  // Show the proposal's people in the quilt: pending rows already carry the amber frame.
  const first = v.new_people[0];
  if (first && app.view.q?.idRow.has(first.id)) app.view.centerOnRow(app.view.q.idRow.get(first.id), true);
}

async function doApprove(id, checks, leftOut, v) {
  const byChange = new Map();
  for (const c of checks) {
    const e = byChange.get(c.change.id) || { change_id: c.change.id, fields: [], overwrite: [], any: false };
    if (c.box.checked) {
      e.any = true;
      if (c.field === 'remove') e.overwrite.push('remove');
      else {
        e.fields.push(c.field);
        if (c.over?.checked) e.overwrite.push(c.field);
      }
    }
    byChange.set(c.change.id, e);
  }
  const decisions = [...byChange.values()].map((e) => ({ change_id: e.change_id, accept: e.any, fields: e.fields.length ? e.fields : null, overwrite: e.overwrite }));
  try {
    const exclude_people = v.new_people.filter((q) => leftOut.has(q.id)).map((q) => q.id);
    const exclude_relationships = v.new_relationships.filter((l) => leftOut.has(l.id)).map((l) => l.id);
    const r = await api(`/proposals/${id}/approve`, { method: 'POST', body: { decisions, exclude_people, exclude_relationships } });
    toast(t('review.approved', { people: r.people_verified, links: r.relationships_verified, changes: r.changes_applied }));
    if (r.people_excluded || r.relationships_excluded) toast(t('review.left_out', { people: r.people_excluded, links: r.relationships_excluded }));
    if (r.affected_proposals?.length) toast(t('review.affected', { n: r.affected_proposals.length }));
    if (r.conflicts_skipped?.length) toast(t('review.skipped', { n: r.conflicts_skipped.length }), 'error');
    if (r.removals_not_confirmed?.length) toast(t('review.removals_left', { n: r.removals_not_confirmed.length }));
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
  }
}

async function doReject(id) {
  const why = await rejectDialog();
  if (!why) return;
  try {
    const r = await api(`/proposals/${id}/reject`, { method: 'POST', body: why });
    toast(t('review.rejected'));
    if (r.affected_proposals?.length) toast(t('review.affected', { n: r.affected_proposals.length }), 'error');
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
  }
}

/** Ask why, so the assistant that sent it can read the reason and not send it again. Resolves to {reason, note} or null. */
function rejectDialog() {
  return new Promise((resolve) => {
    let reason = null;
    const chips = ['wrong', 'duplicate', 'not_wanted', 'not_now'].map((c) =>
      h('button', { class: 'btn tiny', type: 'button', 'aria-pressed': 'false', 'data-reason': c, title: t('reject.hint_' + c), onclick: (e) => {
        reason = reason === c ? null : c;
        for (const b of chips) b.setAttribute('aria-pressed', String(b.dataset.reason === reason));
        e.currentTarget.blur();
      } }, t('reject.' + c)));
    const note = h('textarea', { id: 'reject-note', rows: 2, maxlength: 500, placeholder: t('reject.note_placeholder'), 'aria-label': t('reject.note_placeholder') });
    const d = h('dialog', { class: 'dlg', id: 'dlg-reject' });
    const done = (v) => {
      d.close();
      d.remove();
      resolve(v);
    };
    d.append(h('h2', {}, t('review.reject')), h('p', {}, t('review.reject_body')), h('p', { class: 'muted small' }, t('reject.why')), h('div', { class: 'tools' }, ...chips), note,
      h('div', { class: 'actions' }, h('button', { class: 'btn', onclick: () => done(null) }, t('common.cancel')),
        h('button', { class: 'btn danger', onclick: () => done({ reason, note: note.value.trim() || null }) }, t('review.reject'))));
    d.addEventListener('cancel', () => done(null));
    document.body.append(d);
    d.showModal();
  });
}
