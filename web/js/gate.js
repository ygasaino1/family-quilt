// The screens in front of the tree: first-run wizard, unlock, and (while the app runs) re-locking.
// Nothing else on the page is started until a tree is open.
import { t } from './i18n.js';
import { h, toast, download } from './dom.js';

const MIN_PASSWORD = 8;

async function treeApi(path, body) {
  const res = await fetch('/api/tree' + path, { method: body === undefined ? 'GET' : 'POST', headers: body === undefined ? {} : { 'Content-Type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
  const data = await res.json().catch(() => null);
  if (!res.ok) {
    const e = new Error((data && data.error && data.error.message) || `HTTP ${res.status}`);
    e.code = data && data.error && data.error.code;
    throw e;
  }
  return data;
}
export const getTree = () => treeApi('');
export const treeAction = (name, body = {}) => treeApi('/' + name, body);

let overlay = null;
function show(card) {
  if (!overlay) {
    overlay = h('div', { class: 'gate', role: 'dialog', 'aria-modal': 'true' });
    document.body.append(overlay);
    document.querySelector('.app')?.setAttribute('inert', '');
  }
  overlay.replaceChildren(card);
  overlay.querySelector('input, button')?.focus();
}
function hide() {
  overlay?.remove();
  overlay = null;
  document.querySelector('.app')?.removeAttribute('inert');
}

function field(id, label, attrs = {}) {
  const input = h('input', { id, ...attrs });
  return { input, row: h('div', { class: 'field' }, h('label', { for: id }, label), input) };
}
const errBox = () => h('p', { class: 'err', role: 'alert', id: 'gate-err' });
const say = (box, msg) => (box.textContent = msg || '');

/** Checks a new password pair; returns a message or ''. */
export function passwordProblem(pw, again) {
  if (pw.length < MIN_PASSWORD) return t('gate.pw_short', { n: MIN_PASSWORD });
  if (pw !== again) return t('gate.pw_mismatch');
  return '';
}

/** The recovery key, shown once, with ways to keep it. Resolves when the person says they kept it. */
export function recoveryScreen(key, title) {
  return new Promise((resolve) => {
    const kept = h('input', { type: 'checkbox', id: 'gate-kept' });
    const go = h('button', { class: 'btn primary', id: 'gate-continue', disabled: true, onclick: () => resolve() }, t('common.continue'));
    kept.addEventListener('change', () => (go.disabled = !kept.checked));
    show(h('div', { class: 'gate-card', id: 'gate-recovery' },
      h('h1', {}, t('gate.recovery_title')),
      h('p', {}, t('gate.recovery_body')),
      h('pre', { class: 'recovery', id: 'recovery-key' }, key),
      h('div', { class: 'row' },
        h('button', { class: 'btn small', onclick: () => navigator.clipboard?.writeText(key).then(() => toast(t('settings.copied'))) }, t('settings.copy')),
        h('button', { class: 'btn small', onclick: () => download('family-tree-recovery-key.txt', new Blob([`${title || t('app.name')}\n${t('gate.recovery_file')}\n\n${key}\n`], { type: 'text/plain' })) }, t('gate.save_file')),
        h('button', { class: 'btn small', onclick: () => window.print() }, t('gate.print'))),
      h('p', { class: 'warn' }, t('gate.recovery_warn')),
      h('label', { class: 'check' }, kept, ' ', t('gate.recovery_kept')),
      h('div', { class: 'actions' }, go)));
  });
}

/** Where the tree is kept, with a way to change it before the tree exists (native dialog, or a typed path). */
function whereRow(info) {
  const shown = h('code', { id: 'gate-path' }, info.path);
  const err = h('p', { class: 'err', role: 'alert', id: 'gate-folder-err' });
  const apply = async (folder) => {
    say(err, '');
    try {
      const r = await treeAction('location', { folder });
      shown.textContent = r.path;
      if (r.state !== 'missing' && r.state !== undefined) location.reload(); // an existing tree was found there
    } catch (e) {
      say(err, e.message);
    }
  };
  const typed = h('input', { type: 'text', id: 'gate-folder', placeholder: t('gate.folder_placeholder'), autocomplete: 'off', spellcheck: 'false' });
  const set = h('button', { class: 'btn tiny', id: 'gate-folder-set', type: 'button', onclick: () => typed.value.trim() && apply(typed.value.trim()) }, t('gate.folder_use'));
  const box = h('div', { class: 'row', hidden: true }, typed, set);
  const change = h('button', { class: 'btn tiny', id: 'gate-folder-change', type: 'button' }, t('gate.folder_change'));
  change.addEventListener('click', async () => {
    if (info.can_pick) {
      try {
        const r = await treeAction('pick-folder', {});
        if (r.path) await apply(r.path);
      } catch (e) {
        say(err, e.message);
      }
    } else {
      box.hidden = !box.hidden;
    }
  });
  return { row: h('div', { class: 'field' }, h('span', { class: 'muted small' }, t('gate.where_label'), ' '), shown, ' ', change, box, err) };
}

function wizard(info) {
  const where = whereRow(info);
  return new Promise((resolve) => {
    const name = field('gate-title', t('gate.tree_name'), { maxlength: 200, value: '', autocomplete: 'off' });
    const protect = h('input', { type: 'checkbox', id: 'gate-protect' });
    const pw = field('gate-pw', t('gate.password'), { type: 'password', autocomplete: 'new-password' });
    const pw2 = field('gate-pw2', t('gate.password_again'), { type: 'password', autocomplete: 'new-password' });
    const pwBox = h('div', { id: 'gate-pwbox', hidden: true }, pw.row, pw2.row, h('p', { class: 'muted' }, t('gate.password_hint')));
    protect.addEventListener('change', () => (pwBox.hidden = !protect.checked));
    const file = h('input', { type: 'file', id: 'gate-file', accept: '.ged,.gedcom,text/plain' });
    const err = errBox();
    const go = h('button', { class: 'btn primary', id: 'gate-create' }, t('gate.create'));
    go.addEventListener('click', async () => {
      say(err, '');
      const title = name.input.value.trim();
      if (protect.checked) {
        const p = passwordProblem(pw.input.value, pw2.input.value);
        if (p) return say(err, p);
      }
      go.disabled = true;
      try {
        const r = await treeAction('create', { title, password: protect.checked ? pw.input.value : null });
        if (r.recovery_key) await recoveryScreen(r.recovery_key, title);
        if (file.files[0]) {
          try {
            const res = await fetch('/api/import/gedcom', { method: 'POST', body: await file.files[0].text() });
            const body = await res.json().catch(() => ({}));
            if (!res.ok) throw new Error((body.error && body.error.message) || `HTTP ${res.status}`);
            toast(t('gate.imported', { people: body.people, links: body.links }));
          } catch (e) {
            toast(t('gate.import_failed', { why: e.message }), 'error');
          }
        }
        hide();
        resolve();
      } catch (e) {
        say(err, e.message);
        go.disabled = false;
      }
    });
    show(h('div', { class: 'gate-card', id: 'gate-wizard' },
      h('h1', {}, t('gate.welcome')),
      h('p', { class: 'muted' }, t('gate.welcome_body')),
      name.row,
      h('div', { class: 'field' }, h('label', { class: 'check' }, protect, ' ', t('gate.protect')), h('span', { class: 'muted' }, t('gate.protect_hint'))),
      pwBox,
      h('div', { class: 'field' }, h('label', { for: 'gate-file' }, t('gate.import')), file, h('span', { class: 'muted' }, t('gate.import_hint'))),
      where.row,
      err,
      h('div', { class: 'actions' }, go)));
  });
}

function unlockScreen() {
  return new Promise((resolve) => {
    const pw = field('gate-pw', t('gate.password'), { type: 'password', autocomplete: 'current-password' });
    const err = errBox();
    const go = h('button', { class: 'btn primary', id: 'gate-unlock' }, t('gate.unlock'));
    const submit = async () => {
      say(err, '');
      go.disabled = true;
      try {
        await treeAction('unlock', { password: pw.input.value });
        hide();
        resolve();
      } catch (e) {
        say(err, e.code === 'wrong_password' ? t('gate.wrong_password') : e.message);
        go.disabled = false;
        pw.input.select();
      }
    };
    go.addEventListener('click', submit);
    pw.input.addEventListener('keydown', (e) => e.key === 'Enter' && submit());
    show(h('div', { class: 'gate-card', id: 'gate-unlock-card' },
      h('h1', {}, t('gate.locked_title')),
      h('p', { class: 'muted' }, t('gate.locked_body')),
      pw.row, err,
      h('div', { class: 'actions' }, h('button', { class: 'btn', id: 'gate-forgot', onclick: () => forgotScreen().then(resolve) }, t('gate.forgot')), go)));
  });
}

function forgotScreen() {
  return new Promise((resolve) => {
    const key = field('gate-rk', t('gate.recovery_key'), { autocomplete: 'off', spellcheck: 'false' });
    const pw = field('gate-pw', t('gate.new_password'), { type: 'password', autocomplete: 'new-password' });
    const pw2 = field('gate-pw2', t('gate.password_again'), { type: 'password', autocomplete: 'new-password' });
    const err = errBox();
    const go = h('button', { class: 'btn primary', id: 'gate-reset' }, t('gate.reset'));
    go.addEventListener('click', async () => {
      say(err, '');
      const p = passwordProblem(pw.input.value, pw2.input.value);
      if (p) return say(err, p);
      go.disabled = true;
      try {
        await treeAction('reset', { recovery_key: key.input.value.trim(), new_password: pw.input.value });
        hide();
        resolve();
      } catch (e) {
        say(err, e.code === 'wrong_password' || e.code === 'key_file' ? t('gate.wrong_recovery') : e.message);
        go.disabled = false;
      }
    });
    show(h('div', { class: 'gate-card', id: 'gate-forgot-card' },
      h('h1', {}, t('gate.forgot_title')),
      h('p', { class: 'muted' }, t('gate.forgot_body')),
      key.row, pw.row, pw2.row, err,
      h('div', { class: 'actions' }, h('button', { class: 'btn', onclick: () => unlockScreen().then(resolve) }, t('common.back')), go)));
  });
}

/** Resolves with the tree's info once a tree is open (after the wizard or the unlock screen if needed). */
export async function ensureOpen() {
  const info = await getTree();
  if (info.state === 'open') return info;
  if (info.state === 'missing') await wizard(info);
  else await unlockScreen();
  return getTree();
}

/** The tree was locked while the app was running (idle timer or the Lock button): ask again, then start over. */
let relocking = false;
export async function relock() {
  if (relocking) return;
  relocking = true;
  delete document.documentElement.dataset.ready; // the page is not usable until it has started again
  try {
    await ensureOpen();
    location.reload();
  } finally {
    relocking = false;
  }
}
export { hide as hideGate };
