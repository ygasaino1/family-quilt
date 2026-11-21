// The review screen: what an AI assistant proposed, in your words and theirs, with before/after.
import { t } from './i18n.js';
import { api } from './api.js';
import { h, toast, confirmBox, sexWord, fill } from './dom.js';
import { Quilt } from './quilt.js';
import { QuiltView } from './view.js';

let app;
let selected = null;
let miniView = null;
const root = () => document.getElementById('review-root');

export function init(a) {
  app = a;
}

export function onDataChanged() {
  if (!document.getElementById('screen-review').hidden) show(selected);
}

const val = (v) => (v === null || v === undefined || v === '' ? h('i', { class: 'muted' }, t('review.empty')) : typeof v === 'object' ? JSON.stringify(v) : String(v));
const fieldName = (f) => {
  const k = 'field.' + f;
  return t(k);
};

export async function show(id) {
  selected = id || selected;
  let list;
  try {
    list = await api('/proposals?status=pending');
  } catch (e) {
    return root().replaceChildren(h('p', { class: 'conflict' }, e.message));
  }
  const props = list.proposals;
  if (!props.length) {
    selected = null;
    return root().replaceChildren(h('div', { class: 'card' }, h('h2', {}, t('review.title')), h('p', {}, t('review.none'))));
  }
  if (!props.some((p) => p.id === selected)) selected = props[0].id;
  const left = h('div', { class: 'card', style: 'max-inline-size:none' }, h('h2', {}, t('review.pending_n', { n: props.length })),
    h('div', { class: 'list' }, props.map((p) => h('button', { class: 'item', 'aria-current': p.id === selected ? 'true' : null, onclick: () => show(p.id) }, h('b', {}, p.summary || t('review.no_summary')), h('span', { class: 'muted' }, p.created_by.replace('agent:', '') + ' · ' + String(p.created_at).slice(0, 16).replace('T', ' '))))));
  const detail = h('div', { id: 'review-detail' }, t('common.loading'));
  root().replaceChildren(left, detail);
  await renderDetail(detail, selected);
}

async function renderDetail(box, id) {
  let v;
  try {
    v = await api('/proposals/' + id);
  } catch (e) {
    return box.replaceChildren(h('p', { class: 'conflict' }, e.message));
  }
  const p = v.proposal;
  const blocked = v.depends_on.filter((d) => d.status === 'pending');
  const checks = []; // {change, field, box, conflict, overwriteBox}

  const yours = h('div', {}, h('h3', {}, t('review.your_words')), p.source_text ? h('div', { class: 'quote' }, p.source_text) : h('p', { class: 'muted' }, t('review.no_source')));
  const theirs = h('div', {}, h('h3', {}, t('review.their_understanding')), h('p', {}, p.summary || ''), h('p', { class: 'muted' }, t('review.counts', { people: v.new_people.length, links: v.new_relationships.length, changes: v.changes.length })));

  const newPeople = v.new_people.length
    ? h('div', { class: 'card' }, h('h3', {}, t('review.new_people')), h('ul', { class: 'linklist' }, v.new_people.map((q) => h('li', {}, h('span', { class: 'pill pending' }, t('status.pending')), ' ', h('b', {}, q.display_name), ' ', h('span', { class: 'muted' }, [t('sex.' + sexWord(q.sex)), q.birth ? '* ' + String(q.birth.date ?? q.birth) : '', q.death ? '† ' + String(q.death.date ?? q.death) : ''].filter(Boolean).join(' · ')), q.agent_note ? h('div', { class: 'muted' }, q.agent_note) : null))))
    : null;
  const newLinks = v.new_relationships.length
    ? h('div', { class: 'card' }, h('h3', {}, t('review.new_links')), h('ul', { class: 'linklist' }, v.new_relationships.map((r) => h('li', {}, r.rel_type === 'parent_child' ? t('review.link_parent', { a: r.a_name, b: r.b_name, kind: t('kind.' + (r.kind || 'unknown')) }) : t('review.link_partner', { a: r.a_name, b: r.b_name, state: t('state.' + (r.subtype || 'unknown') + '.long') })))))
    : null;

  const changes = v.changes.map((c) => {
    const rows = [];
    if (c.op === 'remove') {
      const cb = h('input', { type: 'checkbox', id: 'rm-' + c.id });
      checks.push({ change: c, field: 'remove', box: cb, conflict: false });
      return h('div', { class: 'card' }, h('h3', {}, t('review.remove_title', { name: c.target_label })),
        h('div', { class: 'warn' }, t('review.remove_warning')),
        h('label', {}, cb, ' ', t('review.remove_tick')),
        c.stale ? h('p', { class: 'conflict' }, t('review.stale')) : null);
    }
    for (const f of c.fields) {
      const accept = h('input', { type: 'checkbox', id: `f-${c.id}-${f.field}`, checked: !f.conflict });
      const over = f.conflict ? h('input', { type: 'checkbox', id: `o-${c.id}-${f.field}` }) : null;
      checks.push({ change: c, field: f.field, box: accept, conflict: f.conflict, over });
      rows.push(h('tr', {}, h('td', {}, h('label', {}, accept, ' ', fieldName(f.field))), h('td', { class: 'from' }, val(f.from)), h('td', { class: 'to' }, val(f.to)),
        h('td', {}, f.conflict ? h('div', { class: 'conflict' }, t('review.conflict', { current: String(f.current ?? '') }), h('label', {}, over, ' ', t('review.overwrite'))) : '')));
    }
    return h('div', { class: 'card' }, h('h3', {}, c.target_label), c.target_missing ? h('p', { class: 'conflict' }, t('review.missing')) : null,
      h('table', { class: 'diff' }, h('thead', {}, h('tr', {}, h('th', {}, t('review.col_field')), h('th', {}, t('review.col_before')), h('th', {}, t('review.col_after')), h('th', {}))), h('tbody', {}, rows)));
  });

  const warns = v.warnings.length ? h('div', { class: 'card' }, h('h3', {}, t('review.warnings')), v.warnings.map((w) => h('div', { class: 'warn' }, w.message))) : null;
  const deps = blocked.length ? h('div', { class: 'warn', id: 'review-blocked' }, t('review.blocked'), ' ', blocked.map((d) => h('button', { class: 'link', onclick: () => show(d.proposal_id) }, d.summary || d.proposal_id))) : null;

  const quiltBox = h('div', { class: 'review-quilt' }, h('canvas', { id: 'review-canvas' }));
  const approve = h('button', { class: 'btn primary', id: 'review-approve', disabled: blocked.length > 0, onclick: () => doApprove(id, checks) }, t('review.approve'));
  const reject = h('button', { class: 'btn danger', id: 'review-reject', onclick: () => doReject(id) }, t('review.reject'));

  fill(box,
    h('div', { class: 'card' }, h('h2', {}, p.summary || t('review.no_summary')), h('p', { class: 'muted' }, p.created_by.replace('agent:', '') + ' · ' + String(p.created_at).slice(0, 16).replace('T', ' ')), h('div', { class: 'two' }, yours, theirs)),
    deps, warns,
    h('div', { class: 'card' }, h('h3', {}, t('review.picture')), quiltBox),
    newPeople, newLinks, ...changes,
    h('div', { class: 'actions', style: 'max-inline-size:64rem' }, reject, approve),
  );
  // The picture: only this proposal's people and their direct relatives.
  try {
    if (miniView) miniView.dispose?.();
    miniView = new QuiltView(document.getElementById('review-canvas'));
    miniView.readColors();
    const payload = await api('/quilt?review=' + encodeURIComponent(id));
    miniView.setQuilt(new Quilt(payload), { fit: true });
    miniView.fit(false);
  } catch (e) {
    console.warn(e);
  }
}

async function doApprove(id, checks) {
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
    const r = await api(`/proposals/${id}/approve`, { method: 'POST', body: { decisions } });
    toast(t('review.approved', { people: r.people_verified, links: r.relationships_verified, changes: r.changes_applied }));
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
