// The side panel: profile, editing, adding relatives, link kinds, history/undo, "how are we related?".
import { t, fmtDate } from './i18n.js';
import { api, ApiError } from './api.js';
import { h, toast, confirmBox, sexWord, sexLetter, fill } from './dom.js';

let app;
let current = null; // selection currently shown
const side = () => document.getElementById('side');
const KINDS = ['birth', 'adopted', 'foster', 'step', 'guardian', 'sealing', 'unknown'];
const STATES = ['married', 'unmarried', 'divorced', 'ended', 'unknown'];
const QUALS = ['exact', 'about', 'before', 'after', 'estimated'];

export function init(a) {
  app = a;
  app.view.addEventListener('select', (e) => render(e.detail));
  app.openPerson = openPerson;
  document.addEventListener('keydown', (e) => e.key === 'Escape' && !side().hidden && document.activeElement?.closest('#side') && close());
}

export function refresh() {
  if (current && !side().hidden) render(app.view.sel);
}

function close() {
  app.view.clearSelection();
  side().hidden = true;
}

export function openPerson(id) {
  const v = app.view;
  const r = v.q.idRow.get(id);
  v.setSelection([{ kind: 'row', ids: [id] }]);
  if (r !== undefined) v.centerOnRow(r, true);
}

async function render(sel) {
  current = sel;
  const el = side();
  if (!sel.length) {
    el.hidden = true;
    return;
  }
  el.hidden = false;
  const people = [...new Set(sel.flatMap((s) => s.ids))];
  if (sel[0].kind === 'col' && sel.length === 1) return renderFamily(el, sel[0]);
  if (people.length === 2) return renderPair(el, people);
  if (people.length > 2) return renderMany(el, people);
  await renderPerson(el, people[0]);
}

function head(title, id = 'side-title') {
  return h('div', { class: 'side-head' }, h('h2', { id }, title), h('button', { class: 'btn tiny', 'aria-label': t('common.close'), title: t('common.close'), onclick: close }, '×'));
}

// Collapsible sections, like the panels of a properties editor; what is open is remembered between selections.
const closedByDefault = new Set(['history']);
const openState = new Map();
function panel(key, title, count, ...body) {
  const open = openState.has(key) ? openState.get(key) : !closedByDefault.has(key);
  const d = h('details', { class: 'pnl' }, h('summary', {}, h('span', {}, title), count ? h('span', { class: 'cnt' }, String(count)) : null), h('div', { class: 'pnl-body' }, body));
  d.open = open;
  d.addEventListener('toggle', () => openState.set(key, d.open));
  return d;
}

function name(id) {
  const r = app.view.q.idRow.get(id);
  return r === undefined ? id : app.view.q.people[r].n;
}

function renderMany(el, people) {
  el.replaceChildren(head(t('side.selected', { n: people.length })), h('ul', { class: 'linklist' }, people.slice(0, 50).map((id) => h('li', {}, h('button', { class: 'link', onclick: () => openPerson(id) }, name(id))))));
}

function renderFamily(el, s) {
  const q = app.view.q;
  const j = q.colIndexByKey.get(s.colKey);
  if (j === undefined) return (el.hidden = true);
  const col = q.cols[j];
  fill(el,
    head(col.p.map((r) => q.people[r].n).join(' + ') || t('tip.family')),
    col.h !== 'none' ? h('p', {}, t('state.' + col.h + '.long')) : null,
    h('h3', {}, t('side.children')),
    h('ul', { class: 'linklist' }, col.c.map((c) => h('li', {}, h('button', { class: 'link', onclick: () => openPerson(q.people[c.r].id) }, q.people[c.r].n, c.o ? ' (' + t('tip.non_birth') + ')' : '')))),
  );
}

async function renderPair(el, ids) {
  el.replaceChildren(head(t('side.how_related')), h('p', { class: 'muted' }, t('common.loading')));
  try {
    const r = await api(`/relationship?a=${ids[0]}&b=${ids[1]}&pending=1`);
    const kids = [head(t('side.how_related'))];
    kids.push(h('p', {}, h('b', {}, r.a.name), ' / ', h('b', {}, r.b.name)));
    if (r.kind === 'none' || !r.a_is) kids.push(h('p', {}, t('rel.none')));
    else {
      kids.push(h('p', {}, t('rel.a_is', { a: r.a.name, rel: r.a_is, b: r.b.name })));
      kids.push(h('p', { class: 'muted' }, r.by_blood ? t('rel.blood') : t('rel.not_blood')));
      if (r.has_non_birth_link) kids.push(h('p', { class: 'muted' }, t('rel.non_birth_note')));
      if (r.waiting_for_review) kids.push(h('p', { class: 'warn' }, t('rel.waiting')));
    }
    el.replaceChildren(...kids);
  } catch (e) {
    el.replaceChildren(head(t('side.how_related')), h('p', { class: 'conflict' }, e.message));
  }
}

async function renderPerson(el, id) {
  el.replaceChildren(head(name(id)), h('p', { class: 'muted' }, t('common.loading')));
  let d;
  try {
    d = await api('/people/' + id);
  } catch (e) {
    return el.replaceChildren(head(name(id)), h('p', { class: 'conflict' }, e.message));
  }
  if (current?.[0]?.ids?.[0] !== id && current?.length !== 1) return;
  const p = d.person;
  const when = (f) => (f && f.date !== undefined ? fmtDate(String(f.date), f.qualifier) : f ? fmtDate(String(f), 'exact') : '');
  const place = (x) => (x && (x.name || x)) || '';
  const dl = h('dl', { class: 'props' });
  const add = (k, v) => v && dl.append(h('dt', {}, t(k)), h('dd', {}, v));
  add('f.sex', t('sex.' + sexWord(p.sex)));
  add('f.born', [when(p.birth), place(p.birth_place)].filter(Boolean).join(' · '));
  add('f.died', [when(p.death), place(p.death_place)].filter(Boolean).join(' · '));
  add('f.status', p.status === 'pending' ? t('status.pending') : '');
  add('f.note', p.note);
  const group = (role) => d.links.filter((l) => l.role === role);
  const linkRow = (l) => {
    const kindSel =
      l.role === 'partner'
        ? h('select', { class: 'sel', 'aria-label': t('f.state'), onchange: (e) => patchLink(l.link_id, { subtype: e.target.value }) }, STATES.map((s) => h('option', { value: s, selected: l.subtype === s }, t('state.' + s + '.long'))))
        : h('select', { class: 'sel', 'aria-label': t('f.kind'), onchange: (e) => changeKind(l, id, e.target.value, e.target) }, KINDS.map((k) => h('option', { value: k, selected: l.kind === k }, t('kind.' + k))));
    return h('li', { class: 'lrow' }, h('button', { class: 'link', onclick: () => openPerson(l.other_id) }, l.other_name, l.status === 'pending' ? ' ●' : ''), kindSel, h('button', { class: 'btn tiny danger-ghost', 'aria-label': t('side.remove_link'), title: t('side.remove_link'), onclick: () => removeLink(l) }, '×'));
  };
  const section = (key, title, list) => (list.length ? [panel(key, t(title), list.length, h('ul', { class: 'linklist' }, list.map(linkRow)))] : []);
  const pending = d.suggested_changes?.length ? h('div', { class: 'warn' }, t('side.suggested', { n: d.suggested_changes.length }), ' ', h('button', { class: 'link', onclick: () => app.showScreen('review') }, t('nav.review'))) : null;
  fill(el,
    head(p.display_name),
    pending,
    panel('details', t('side.details'), 0, dl, h('div', { class: 'tools' }, h('button', { class: 'btn small', onclick: () => personDialog({ edit: d }) }, t('side.edit')), h('button', { class: 'btn small danger-ghost', onclick: () => deletePerson(p) }, t('side.delete')))),
    panel('add', t('side.add_heading'), 0, h('div', { class: 'tools' }, ['parent', 'partner', 'child', 'sibling'].map((role) => h('button', { class: 'btn small', onclick: () => personDialog({ role, anchor: d }) }, '+ ' + t('side.add_' + role))))),
    ...section('parents', 'side.parents', group('parent')),
    ...section('partners', 'side.partners', group('partner')),
    ...section('children', 'side.children', group('child')),
    panel('history', t('side.history'), 0, h('div', { id: 'hist' }, t('common.loading'))),
  );
  loadHistory(id);
}

async function loadHistory(id) {
  const box = document.getElementById('hist');
  try {
    const { revisions } = await api(`/history?target_type=person&target_id=${id}&limit=10`);
    box?.replaceChildren(
      ...(revisions.length
        ? revisions.map((r) => h('div', { class: 'lrow' }, h('span', { class: 'muted' }, `${String(r.at).slice(0, 16).replace('T', ' ')} · ${r.by}`), h('button', { class: 'btn small', onclick: () => undo(r.id) }, t('side.undo'))))
        : [h('span', { class: 'muted' }, t('side.no_history'))]),
    );
  } catch (e) {
    box?.replaceChildren(h('span', { class: 'conflict' }, e.message));
  }
}

async function undo(revId) {
  try {
    await api(`/history/${revId}/undo`, { method: 'POST' });
    toast(t('toast.undone'));
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
  }
}

async function patchLink(id, body) {
  try {
    await api('/relationships/' + id, { method: 'PATCH', body });
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
    refresh();
  }
}

async function changeKind(l, personId, kind, select) {
  const wasBlood = l.kind === 'birth';
  const willBlood = kind === 'birth';
  if (wasBlood !== willBlood && !(await confirmBox(t('side.kind_title'), t('side.kind_impact'), { ok: t('side.kind_ok') }))) {
    select.value = l.kind;
    return;
  }
  await patchLink(l.link_id, { kind });
}

async function removeLink(l) {
  if (!(await confirmBox(t('side.remove_link'), t('side.remove_link_body', { name: l.other_name }), { danger: true, ok: t('side.remove_link') }))) return;
  try {
    await api('/relationships/' + l.link_id, { method: 'DELETE' });
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
  }
}

async function deletePerson(p) {
  if (!(await confirmBox(t('side.delete'), t('side.delete_body', { name: p.display_name }), { danger: true, ok: t('side.delete') }))) return;
  try {
    const r = await api('/people/' + p.id, { method: 'DELETE' });
    toast(t('toast.deleted', { n: r.links_removed }));
    side().hidden = true;
    app.view.clearSelection();
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
  }
}

export function addFirstPerson() {
  personDialog({});
}

// ---------- the add/edit dialog ----------
function field(id, label, input, err) {
  return h('div', { class: 'field' }, h('label', { for: id }, label), input, h('span', { class: 'err', id: id + '-err' }, err || ''));
}

function personDialog({ edit, role, anchor }) {
  const dlg = document.getElementById('dlg-person');
  const p = edit?.person;
  const nm = p?.names?.find((n) => n.is_primary) || p?.names?.[0];
  const dateOf = (f) => (f ? String(f.date ?? f) : '');
  const inputs = {
    given: h('input', { id: 'pf-given', value: nm?.given || '', autocomplete: 'off' }),
    family: h('input', { id: 'pf-family', value: nm?.family || '', autocomplete: 'off' }),
    full: h('input', { id: 'pf-full', value: nm && !nm.given && !nm.family ? nm.full_name : '', autocomplete: 'off' }),
    sex: h('select', { id: 'pf-sex' }, ['U', 'F', 'M', 'X'].map((s) => h('option', { value: s, selected: sexLetter(p?.sex || 'U') === s }, t('sex.' + sexWord(s))))),
    bd: h('input', { id: 'pf-bd', value: dateOf(p?.birth), placeholder: t('f.date_hint') }),
    bq: h('select', { id: 'pf-bq' }, QUALS.map((q) => h('option', { value: q, selected: (p?.birth?.qualifier || 'exact') === q }, t('q.' + q)))),
    dd: h('input', { id: 'pf-dd', value: dateOf(p?.death), placeholder: t('f.date_hint') }),
    dq: h('select', { id: 'pf-dq' }, QUALS.map((q) => h('option', { value: q, selected: (p?.death?.qualifier || 'exact') === q }, t('q.' + q)))),
    note: h('textarea', { id: 'pf-note', rows: 3 }, p?.note || ''),
    kind: h('select', { id: 'pf-kind' }, KINDS.map((k) => h('option', { value: k }, t('kind.' + k)))),
    state: h('select', { id: 'pf-state' }, STATES.map((k) => h('option', { value: k }, t('state.' + k + '.long')))),
  };
  const title = edit ? t('dlg.edit_person') : role ? t('dlg.add_' + role) : t('dlg.new_person');
  const err = h('p', { class: 'conflict', role: 'alert' });
  const form = h('form', { method: 'dialog' },
    h('h2', {}, title),
    anchor ? h('p', { class: 'muted' }, t('dlg.relative_of', { name: anchor.person.display_name })) : null,
    field('pf-given', t('f.given'), inputs.given),
    field('pf-family', t('f.family'), inputs.family),
    field('pf-full', t('f.full_hint'), inputs.full),
    field('pf-sex', t('f.sex'), inputs.sex),
    h('div', { class: 'two' }, field('pf-bd', t('f.born'), inputs.bd), field('pf-bq', t('f.qualifier'), inputs.bq)),
    h('div', { class: 'two' }, field('pf-dd', t('f.died'), inputs.dd), field('pf-dq', t('f.qualifier'), inputs.dq)),
    field('pf-note', t('f.note'), inputs.note),
    role === 'parent' || role === 'child' || role === 'sibling' ? field('pf-kind', t('f.kind'), inputs.kind) : null,
    role === 'partner' ? field('pf-state', t('f.state'), inputs.state) : null,
    err,
    h('div', { class: 'actions' }, h('button', { type: 'button', class: 'btn', onclick: () => dlg.close() }, t('common.cancel')), h('button', { type: 'submit', class: 'btn primary', id: 'pf-save' }, t('common.save'))),
  );
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    err.textContent = '';
    try {
      await save();
      dlg.close();
      await app.refreshAll();
    } catch (ex) {
      err.textContent = ex instanceof ApiError && ex.body?.errors ? ex.body.errors.map((x) => `${x.path}: ${x.message}`).join('; ') : ex.message;
    }
  });
  async function save() {
    const fields = {
      sex: inputs.sex.value,
      birth_date: inputs.bd.value.trim() || null,
      birth_qualifier: inputs.bd.value.trim() ? inputs.bq.value : null,
      death_date: inputs.dd.value.trim() || null,
      death_qualifier: inputs.dd.value.trim() ? inputs.dq.value : null,
      note: inputs.note.value.trim() || null,
    };
    const nameFields = { given: inputs.given.value.trim() || null, family: inputs.family.value.trim() || null, full_name: inputs.full.value.trim() || [inputs.given.value, inputs.family.value].map((x) => x.trim()).filter(Boolean).join(' ') };
    if (!nameFields.full_name) throw new Error(t('dlg.name_required'));
    if (edit) {
      await api('/people/' + p.id, { method: 'PATCH', body: fields });
      if (nm) await api('/names/' + nm.id, { method: 'PATCH', body: nameFields });
      return;
    }
    if (role === 'sibling' && !anchor.links.some((l) => l.role === 'parent')) throw new Error(t('dlg.sibling_needs_parent'));
    const created = await api('/people', { method: 'POST', body: { ...fields, given_name: nameFields.given, family_name: nameFields.family, full_name: nameFields.full_name } });
    if (!role) return;
    const a = anchor.person.id;
    const c = created.id;
    if (role === 'parent') await api('/relationships', { method: 'POST', body: { type: 'parent_child', a: c, b: a, kind: inputs.kind.value } });
    else if (role === 'child') await api('/relationships', { method: 'POST', body: { type: 'parent_child', a, b: c, kind: inputs.kind.value } });
    else if (role === 'partner') await api('/relationships', { method: 'POST', body: { type: 'partnership', a, b: c, subtype: inputs.state.value } });
    else if (role === 'sibling') {
      const parents = anchor.links.filter((l) => l.role === 'parent');
      if (!parents.length) {
        await api('/people/' + c, { method: 'DELETE' });
        throw new Error(t('dlg.sibling_needs_parent'));
      }
      for (const l of parents) await api('/relationships', { method: 'POST', body: { type: 'parent_child', a: l.other_id, b: c, kind: inputs.kind.value } });
    }
  }
  dlg.replaceChildren(form);
  dlg.showModal();
  inputs.given.focus();
}
