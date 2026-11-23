// The review panel, docked beside the quilt: what an AI assistant proposed, in your words and theirs.
import { t, fmtAgo } from './i18n.js';
import { api } from './api.js';
import { h, toast, confirmBox, sexWord, fill, icon } from './dom.js';

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
const born = (q) => (q.birth ? String(q.birth.date ?? q.birth) : '');
const WARN_CODES = ['parent_too_young', 'parent_too_old', 'parent_died_before_birth', 'death_before_birth', 'possible_duplicate', 'link_kind_impact', 'person_removal_impact'];

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
  const leftOut = new Set();
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
  for (const q of v.new_people) {
    const r = { kind: 'person', id: q.id };
    const btn = leaveOutBtn(r);
    r.el = h('div', { class: 'item-row' }, h('span', { class: 'waiting' }, t('review.waiting')), h('span', { class: 'nm' }, q.display_name, born(q) ? ` · ${born(q)} –` : ''),
      h('button', { class: 'link', title: t('review.show_in_tree'), 'aria-label': t('review.show_in_tree'), onclick: () => app.view.q && app.openPerson(q.id) }, icon('tree', 16)), btn);
    items.push(r.el);
  }
  for (const l of v.new_relationships) {
    const r = { kind: 'link', id: l.id, a: l.person_a, b: l.person_b };
    const btn = leaveOutBtn(r);
    r.el = h('div', { class: 'item-row' }, h('span', { class: 'waiting' }, t('review.waiting')), h('span', { class: 'nm' },
      l.rel_type === 'parent_child' ? t('review.link_parent', { a: l.a_name, b: l.b_name, kind: t('kind.' + (l.kind || 'unknown')) }) : t('review.link_partner', { a: l.a_name, b: l.b_name, state: t('state.' + (l.subtype || 'unknown') + '.long') })), btn);
    items.push(r.el);
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
      changeBlocks.push(h('div', { class: 'sugg' },
        h('span', { class: 'what' }, c.target_label, ' · ', t('field.' + f.field)),
        h('span', { class: 'from' }, val(f.from)), h('span', { class: 'arrow' }, '→'), h('span', { class: 'to' }, val(f.to)),
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
  if (!(await confirmBox(t('review.reject'), t('review.reject_body'), { danger: true, ok: t('review.reject') }))) return;
  try {
    const r = await api(`/proposals/${id}/reject`, { method: 'POST' });
    toast(t('review.rejected'));
    if (r.affected_proposals?.length) toast(t('review.affected', { n: r.affected_proposals.length }), 'error');
    await app.refreshAll();
  } catch (e) {
    toast(e.message, 'error');
  }
}
