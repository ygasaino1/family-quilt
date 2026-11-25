import { t } from './i18n.js';
import { api } from './api.js';
import { h, toast, fill } from './dom.js';
import { pref, setPref } from './prefs.js';
import { applyAppearance } from './main.js';
import { getTree, treeAction, passwordProblem, recoveryScreen, hideGate } from './gate.js';

let app;
export function init(a) {
  app = a;
}

function pick(id, label, key, options) {
  const sel = h('select', { id }, options.map(([v, text]) => h('option', { value: v, selected: pref(key) === v }, text)));
  sel.addEventListener('change', () => {
    setPref(key, sel.value);
    if (key === 'lang') location.reload();
    else applyAppearance();
  });
  return h('div', { class: 'field' }, h('label', { for: id }, label), sel);
}

export async function open({ agent = false } = {}) {
  const dlg = document.getElementById('dlg-settings');
  const s = await api('/settings').catch(() => ({ tree_title: '' }));
  const status = app.status || (await api('/status'));
  const title = h('input', { id: 'set-title', value: s.tree_title || '', maxlength: 200 });
  title.addEventListener('change', async () => {
    try {
      await api('/settings', { method: 'PUT', body: { tree_title: title.value } });
      app.setTitle(title.value);
    } catch (e) {
      toast(e.message, 'error');
    }
  });
  const privacy = h('input', { type: 'checkbox', id: 'set-privacy', checked: pref('privacy') === '1' });
  privacy.addEventListener('change', () => {
    setPref('privacy', privacy.checked ? '1' : '0');
    applyAppearance();
    app.view.dirty = true;
  });
  const live = h('input', { type: 'checkbox', id: 'set-live', checked: pref('live') === '1' });
  live.addEventListener('change', () => setPref('live', live.checked ? '1' : '0'));
  const security = h('div', { id: 'security' });
  await renderSecurity(security, dlg);
  const agentBox = h('div', { id: 'agent-box' });
  const agentBtn = h('button', { class: 'btn', id: 'agent-show', onclick: () => loadAgent(agentBox, agentBtn) }, t('settings.agent_show'));
  dlg.replaceChildren(
    h('h2', {}, t('settings.title')),
    h('div', { class: 'field' }, h('label', { for: 'set-title' }, t('settings.tree_title')), title),
    pick('set-theme', t('settings.theme'), 'theme', [['auto', t('settings.theme_auto')], ['light', t('settings.theme_light')], ['dark', t('settings.theme_dark')]]),
    pick('set-scale', t('settings.text_size'), 'scale', [['1', t('settings.size_normal')], ['1.15', t('settings.size_large')], ['1.3', t('settings.size_larger')], ['1.5', t('settings.size_largest')]]),
    pick('set-lang', t('settings.language'), 'lang', [['en', 'English'], ['qps', 'Pseudo (testing)']]),
    pick('set-calendar', t('settings.calendar'), 'calendar', [['gregorian', t('settings.calendar_gregorian')], ['jalali', t('settings.calendar_jalali')]]),
    h('p', { class: 'muted' }, t('settings.calendar_hint')),
    h('div', { class: 'field' }, h('label', {}, privacy, ' ', t('settings.privacy')), h('span', { class: 'muted' }, t('settings.privacy_hint'))),
    h('div', { class: 'field' }, h('label', {}, live, ' ', t('settings.live')), h('span', { class: 'muted' }, t('settings.live_hint'))),
    h('div', { class: 'row' }, h('button', { class: 'btn', id: 'set-backup', onclick: () => backup() }, t('settings.backup')), h('span', { class: 'muted' }, t('settings.backup_hint'))),
    h('h3', {}, t('settings.security')),
    security,
    h('h3', {}, t('settings.agent_title')),
    h('p', { class: 'muted' }, t('settings.agent_hint')),
    agentBtn, agentBox,
    h('p', { class: 'muted' }, t('settings.version', { v: status.version })),
    h('div', { class: 'actions' }, h('button', { class: 'btn primary', onclick: () => dlg.close() }, t('common.close'))),
  );
  dlg.showModal();
  if (agent) {
    agentBtn.click();
    agentBox.scrollIntoView?.();
  }
}

async function backup() {
  try {
    const res = await fetch('/api/backup');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const name = /filename="([^"]+)"/.exec(res.headers.get('content-disposition') || '')?.[1] || 'family-tree-backup.sqlite';
    const a = h('a', { href: URL.createObjectURL(await res.blob()), download: name });
    document.body.append(a);
    a.click();
    a.remove();
  } catch (e) {
    toast(e.message, 'error');
  }
}

async function loadAgent(box, btn) {
  btn.hidden = true;
  const a = await api('/settings/agent');
  const copy = (text) => h('button', { class: 'btn small', onclick: () => navigator.clipboard?.writeText(text).then(() => toast(t('settings.copied'))) }, t('settings.copy'));
  const snip = (label, text) => h('div', { class: 'field' }, h('label', {}, label), h('pre', { class: 'snip' }, text), copy(text));
  const test = h('button', { class: 'btn', id: 'agent-test', onclick: async () => {
    try {
      const r = await api('/settings/agent/test', { method: 'POST' });
      toast(r.ok ? t('settings.test_ok', { n: r.tools.length }) : t('settings.test_bad'), r.ok ? 'info' : 'error');
    } catch (e) {
      toast(e.message, 'error');
    }
  } }, t('settings.test'));
  fill(box,
    h('p', { class: 'warn' }, t('settings.agent_warning')),
    snip(t('settings.snip_desktop'), a.claude_desktop_config),
    snip(t('settings.snip_code_http'), a.claude_code_http),
    snip(t('settings.snip_code_stdio'), a.claude_code_stdio),
    snip(t('settings.token'), a.token),
    test,
    a.last_activity?.length ? h('div', {}, h('h3', {}, t('settings.activity')), a.last_activity.map((x) => h('div', { class: 'muted' }, JSON.stringify(x)))) : null,
  );
}

/** Lock now, auto-lock, and setting, changing or removing the password. */
async function renderSecurity(box, dlg) {
  const info = await getTree();
  const again = () => renderSecurity(box, dlg).catch((e) => toast(e.message, 'error'));
  const fieldRow = (id, label, autocomplete) => {
    const input = h('input', { id, type: 'password', autocomplete });
    return { input, row: h('div', { class: 'field' }, h('label', { for: id }, label), input) };
  };
  const form = (fields, submitLabel, run) => {
    const err = h('p', { class: 'err', role: 'alert' });
    const go = h('button', { class: 'btn primary', onclick: async () => {
      err.textContent = '';
      go.disabled = true;
      try {
        await run();
      } catch (e) {
        err.textContent = e.code === 'wrong_password' ? t('gate.wrong_password') : e.message;
        go.disabled = false;
      }
    } }, submitLabel);
    return h('div', { class: 'subform' }, fields.map((f) => f.row), err, h('div', { class: 'actions' }, go));
  };
  const show = (el) => fill(slot, el);
  const slot = h('div', { id: 'security-form' });
  const parts = [];
  if (info.protected) {
    const sel = h('select', { id: 'set-autolock' }, [[0, t('settings.autolock_never')], [5, '5'], [15, '15'], [30, '30'], [60, '60']].map(([v, text]) => h('option', { value: v, selected: info.auto_lock_minutes === v }, v ? t('settings.autolock_min', { n: text }) : text)));
    sel.addEventListener('change', async () => {
      try {
        await treeAction('auto-lock', { minutes: +sel.value });
      } catch (e) {
        toast(e.message, 'error');
      }
    });
    const lockNow = h('button', { class: 'btn', id: 'set-lock', onclick: async () => {
      dlg.close();
      await treeAction('lock');
      window.dispatchEvent(new Event('tree-locked'));
    } }, t('settings.lock_now'));
    const cur = fieldRow('pw-current', t('gate.current_password'), 'current-password');
    const n1 = fieldRow('pw-new', t('gate.new_password'), 'new-password');
    const n2 = fieldRow('pw-new2', t('gate.password_again'), 'new-password');
    const change = h('button', { class: 'btn', id: 'set-change-pw', onclick: () => show(form([cur, n1, n2], t('settings.change_password'), async () => {
      const p = passwordProblem(n1.input.value, n2.input.value);
      if (p) throw new Error(p);
      await treeAction('password', { current: cur.input.value, new: n1.input.value });
      toast(t('settings.password_changed'));
      show(null);
    })) }, t('settings.change_password'));
    const c2 = fieldRow('pw-current2', t('gate.current_password'), 'current-password');
    const remove = h('button', { class: 'btn danger-ghost', id: 'set-remove-pw', onclick: () => show(h('div', {}, h('p', { class: 'warn' }, t('settings.remove_warning')), form([c2], t('settings.remove_password'), async () => {
      await treeAction('unprotect', { current: c2.input.value });
      window.dispatchEvent(new CustomEvent('tree-protection', { detail: false }));
      toast(t('settings.password_removed'));
      await again();
    }))) }, t('settings.remove_password'));
    parts.push(h('div', { class: 'row' }, lockNow, change, remove), h('div', { class: 'field' }, h('label', { for: 'set-autolock' }, t('settings.autolock')), sel), slot);
  } else {
    const n1 = fieldRow('pw-new', t('gate.password'), 'new-password');
    const n2 = fieldRow('pw-new2', t('gate.password_again'), 'new-password');
    const set = h('button', { class: 'btn', id: 'set-add-pw', onclick: () => show(h('div', {}, h('p', { class: 'muted' }, t('gate.password_hint')), form([n1, n2], t('settings.set_password'), async () => {
      const p = passwordProblem(n1.input.value, n2.input.value);
      if (p) throw new Error(p);
      const r = await treeAction('protect', { new: n1.input.value });
      dlg.close();
      await recoveryScreen(r.recovery_key, document.getElementById('title').textContent);
      hideGate();
      window.dispatchEvent(new CustomEvent('tree-protection', { detail: true }));
      toast(t('settings.password_set'));
    }))) }, t('settings.set_password'));
    parts.push(h('p', { class: 'muted' }, t('settings.unprotected')), h('div', { class: 'row' }, set), slot);
  }
  fill(box, parts);
}
