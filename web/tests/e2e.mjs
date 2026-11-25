// End-to-end checks in a real browser (Playwright, development only: nothing from it ships).
// Run: FAMILY_APP=target/debug/family-app node web/tests/e2e.mjs   (needs `playwright` resolvable)
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_PATH || 'playwright');

const exe = process.env.FAMILY_APP || 'target/debug/family-app';
const out = path.resolve(process.env.E2E_OUT || 'web/tests/out');
fs.mkdirSync(out, { recursive: true });
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fq-e2e-'));
const port = 20000 + Math.floor(Math.random() * 20000);
const base = `http://127.0.0.1:${port}`;
const srv = spawn(exe, ['--data', dir, '--headless', '--port', String(port), '--fast-kdf-for-tests'], { stdio: ['ignore', 'ignore', 'pipe'], env: { ...process.env, FAMILY_PORT_AUTO_MAX: '300' } });
let srvLog = '';
srv.stderr.on('data', (d) => (srvLog += d));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const results = [];
async function check(name, fn) {
  try {
    await fn();
    results.push('ok   ' + name);
  } catch (e) {
    failures++;
    results.push('FAIL ' + name + '\n     ' + String(e.message).split('\n').join('\n     '));
  }
}

const ui = async (p, method = 'GET', body) => {
  const res = await fetch(base + '/api' + p, { method, headers: { 'Sec-Fetch-Site': 'same-origin', ...(body ? { 'Content-Type': 'application/json' } : {}) }, body: body ? JSON.stringify(body) : undefined });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${p}: ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
};
const agent = async (p, body) => {
  const token = fs.readFileSync(path.join(dir, 'agent-token.txt'), 'utf8').trim();
  const res = await fetch(base + '/agent/v1' + p, { method: body ? 'POST' : 'GET', headers: { Authorization: 'Bearer ' + token, 'X-Agent-Name': 'e2e-agent', ...(body ? { 'Content-Type': 'application/json' } : {}) }, body: body ? JSON.stringify(body) : undefined });
  const text = await res.text();
  if (!res.ok) throw new Error(`agent ${p}: ${res.status} ${text}`);
  return JSON.parse(text);
};

let browser;
try {
  for (let i = 0; i < 100 && !srvLog.includes('server on'); i++) await sleep(100);
  browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: 'light' });
  const page = await ctx.newPage();
  const warnings = [];
  const errors = [];
  let allow400 = false; // set while a step sends a bad request on purpose
  const ring = [];
  page.on('console', (m) => ring.push(m.type() + ': ' + m.text().slice(0, 160)) && ring.length > 12 && ring.shift());
  page.on('pageerror', (e) => ring.push('PAGEERROR ' + String(e.message).slice(0, 200)) && ring.length > 12 && ring.shift());
  page.on('framenavigated', (f) => f === page.mainFrame() && ring.push('NAV ' + f.url()) && ring.length > 12 && ring.shift());
  page.on('console', (m) => {
    if (m.text().includes('missing translation key')) warnings.push(m.text());
    if (m.type() === 'error' && !/status of 423/.test(m.text()) && !(allow400 && /status of (400|422)/.test(m.text()))) errors.push(m.text()); // 423: the tree was locked on purpose
  });
  page.on('pageerror', (e) => errors.push(e.stack || e.message));
  await check('first run: the wizard makes the tree (name, no password) and the app starts', async () => {
    await page.goto(base + '/');
    await page.waitForSelector('#gate-wizard');
    assert.equal((await ui('/tree')).state, 'missing');
    assert.equal(await page.locator('#gate-pwbox').isHidden(), true);
    // The folder choice: without a native dialog (this build has none) a typed path is offered; a bad one is explained.
    assert.ok((await page.textContent('#gate-path')).includes('tree.sqlite'));
    await page.click('#gate-folder-change');
    await page.fill('#gate-folder', 'not/absolute');
    allow400 = true;
    await page.click('#gate-folder-set');
    await page.waitForFunction(() => /full path/i.test(document.querySelector('#gate-folder-err')?.textContent || ''));
    allow400 = false;
    await page.fill('#gate-title', 'The Roots');
    await page.click('#gate-protect');
    await page.fill('#gate-pw', 'short');
    await page.fill('#gate-pw2', 'short');
    await page.click('#gate-create');
    assert.match(await page.textContent('#gate-err'), /at least 8/);
    await page.click('#gate-protect'); // changed my mind: no password
    await page.click('#gate-create');
    await page.waitForSelector('html[data-ready="1"]');
    assert.equal(await page.textContent('#title'), 'The Roots');
    assert.equal((await ui('/tree')).state, 'open');
  });

  // ---- fixture: Anna + Bob (married); Cy their birth child; Dee adopted by Bob only
  const person = (given, family, sex, birth) => ui('/people', 'POST', { given_name: given, family_name: family, sex, birth_date: birth });
  const anna = await person('Anna', 'Root', 'F', '1950');
  const bob = await person('Bob', 'Root', 'M', '1948');
  const cy = await person('Cy', 'Root', 'M');
  const dee = await person('Dee', 'Root', 'F', '1980');
  await ui('/relationships', 'POST', { type: 'partnership', a: anna.id, b: bob.id, subtype: 'married' });
  await ui('/relationships', 'POST', { type: 'parent_child', a: anna.id, b: cy.id, kind: 'birth' });
  await ui('/relationships', 'POST', { type: 'parent_child', a: bob.id, b: cy.id, kind: 'birth' });
  await ui('/relationships', 'POST', { type: 'parent_child', a: bob.id, b: dee.id, kind: 'adopted' });

  await page.reload();
  await page.waitForSelector('html[data-ready="1"]');
  await page.waitForFunction(() => window.__app.view.q && window.__app.view.q.n === 4);
  await page.waitForFunction(() => document.getElementById('strip').hidden && !window.__app.view.anim, null, { timeout: 15000 });

  // Put the quilt at a readable zoom so cells are easy to hit.
  await page.evaluate(() => {
    const v = window.__app.view;
    v.tf = { k: 1.5, x: 260, y: 70 };
    v.anim = null;
    v.dirty = true;
  });
  await page.waitForTimeout(200);

  const where = (name) =>
    page.evaluate((nm) => {
      const v = window.__app.view;
      const q = v.q;
      const r = q.people.findIndex((p) => p.n.startsWith(nm));
      const j = q.first[r];
      const g = v.geo;
      const rect = v.canvas.getBoundingClientRect();
      return { r, j, x: rect.left + v.tf.x + (g.cx[j] + 10) * v.tf.k, y: rect.top + v.tf.y + (g.ry[r] + 10) * v.tf.k };
    }, name);
  const pixel = (x, y) =>
    page.evaluate(([x, y]) => {
      const v = window.__app.view;
      const rect = v.canvas.getBoundingClientRect();
      const d = v.ctx.getImageData(Math.round((x - rect.left) * v.size.dpr), Math.round((y - rect.top) * v.size.dpr), 1, 1).data;
      return [d[0], d[1], d[2]];
    }, [x, y]);
  const dark = ([r, g, b]) => r + g + b < 250;
  const light = ([r, g, b]) => r + g + b > 600;

  await check('symbols: square = man, circle = woman, filled', async () => {
    const bobAt = await where('Bob');
    assert.ok(dark(await pixel(bobAt.x, bobAt.y)), 'man cell centre is filled');
    // Corner of the square is filled for a man; the corner of a circle is empty.
    const cornerM = await pixel(bobAt.x + 6 * 1.5, bobAt.y + 6 * 1.5);
    assert.ok(dark(cornerM), 'square has its corner ' + JSON.stringify({ cornerM, bobAt, tf: await page.evaluate(() => window.__app.view.tf) }));
    const annaAt = await where('Anna');
    assert.ok(dark(await pixel(annaAt.x, annaAt.y)), 'woman cell centre is filled');
    assert.ok(!dark(await pixel(annaAt.x + 6.5 * 1.5, annaAt.y + 6.5 * 1.5)), 'circle has no corner');
  });

  await check('symbols: non-birth child is hollow (outlined)', async () => {
    const deeCell = await page.evaluate(() => {
      const v = window.__app.view;
      const q = v.q;
      const r = q.people.findIndex((p) => p.n.startsWith('Dee'));
      const j = q.first[r];
      const c = q.cols[j].c.find((x) => x.r === r);
      const g = v.geo;
      const rect = v.canvas.getBoundingClientRect();
      return { outlined: !!c.o, x: rect.left + v.tf.x + (g.cx[j] + 10) * v.tf.k, y: rect.top + v.tf.y + (g.ry[r] + 10) * v.tf.k };
    });
    assert.equal(deeCell.outlined, true);
    assert.ok(light(await pixel(deeCell.x, deeCell.y)), 'hollow centre shows the background');
    let ring = false;
    for (const dx of [-2, -1, 0, 1, 2]) ring = ring || dark(await pixel(deeCell.x + 7.6 * 1.5 + dx, deeCell.y));
    assert.ok(ring, 'hollow has its outline');
  });

  await check('header letter M for the married family', async () => {
    const letter = await page.evaluate(() => window.__app.view.q.cols.map((c) => c.h));
    assert.ok(letter.includes('married'));
    await page.screenshot({ path: path.join(out, 'light-quilt.png') });
  });

  await check('geometry follows the original: staircase, names between column groups, header between parents and children', async () => {
    const bad = await page.evaluate(() => {
      const v = window.__app.view;
      const q = v.q;
      const g = v.geo;
      const out = [];
      q.cols.forEach((col, j) => {
        for (const r of col.p) {
          if (!(g.chy[j] >= g.ry[r] + 20)) out.push(`header of column ${j} is not below parent row ${r}`);
          if (!(g.cx[j] >= g.rnx[r] + g.lo[r] + g.tw[r])) out.push(`column ${j} is not right of the name of parent ${r}`);
        }
        for (const c of col.c) {
          if (!(g.chy[j] + 20 <= g.ry[c.r])) out.push(`header of column ${j} is not above child row ${c.r}`);
          if (!(g.cx[j] + 20 <= g.rnx[c.r])) out.push(`column ${j} is not left of the name of child ${c.r}`);
        }
      });
      // rows and columns are sorted by position (the renderer bisects on them)
      for (let i = 1; i < q.n; i++) if (!(g.ry[i] > g.ry[i - 1])) out.push('rows not ascending at ' + i);
      for (let j = 1; j < q.m; j++) if (!(g.cx[j] > g.cx[j - 1])) out.push('columns not ascending at ' + j);
      return out;
    });
    assert.deepEqual(bad, []);
  });

  await check('selecting a person does not move the view; sidebar opens', async () => {
    const before = await page.evaluate(() => ({ ...window.__app.view.tf }));
    const cyAt = await where('Cy');
    await page.mouse.click(cyAt.x, cyAt.y);
    await page.waitForSelector('#side:not([hidden]) h2');
    const after = await page.evaluate(() => ({ ...window.__app.view.tf }));
    assert.deepEqual(after, before);
    assert.match(await page.textContent('#side h2'), /Cy Root/);
    const sel = await page.evaluate(() => window.__app.view.sel.length);
    assert.equal(sel, 1);
  });

  await check('shift-click adds a second selection with its own colour; "how related" answers', async () => {
    const deeAt = await where('Dee');
    await page.keyboard.down('Shift');
    await page.mouse.click(deeAt.x, deeAt.y);
    await page.keyboard.up('Shift');
    await page.waitForFunction(() => window.__app.view.sel.length === 2, null, { timeout: 5000 }).catch(async (e) => {
      throw new Error('second selection missing: ' + JSON.stringify(await page.evaluate(() => ({ sel: window.__app.view.sel, hover: window.__app.view.hover, tf: window.__app.view.tf, anim: !!window.__app.view.anim, side: !document.getElementById('side').hidden }))) + ' dee at ' + JSON.stringify(deeAt));
    });
    const colours = await page.evaluate(() => window.__app.view.sel.map((s) => s.colour));
    assert.notEqual(colours[0], colours[1]);
    await page.waitForSelector('#side h2');
    assert.match(await page.textContent('#side'), /related|sibling|brother|sister|relationship/i);
  });

  await check('clicking empty space clears the selection', async () => {
    await page.mouse.click(600, 600);
    await page.waitForFunction(() => window.__app.view.sel.length === 0);
    assert.equal(await page.isHidden('#side'), true);
  });

  await check('wheel zooms around the pointer; Fit works; no jump on select afterwards', async () => {
    const k0 = await page.evaluate(() => window.__app.view.tf.k);
    await page.mouse.move(500, 400);
    await page.mouse.wheel(0, -300);
    await page.waitForTimeout(100);
    const k1 = await page.evaluate(() => window.__app.view.tf.k);
    assert.ok(k1 > k0);
    await page.click('#fit');
    await page.waitForTimeout(400);
  });

  await check('search finds a person and selects them', async () => {
    await page.fill('#search', 'dee');
    assert.match(await page.textContent('#search-count'), /1 of 1/);
    await page.press('#search', 'Enter');
    await page.waitForFunction(() => window.__app.view.sel.length === 1);
    await page.fill('#search', '');
    await page.mouse.click(600, 700);
  });

  // ---- an AI assistant proposes things
  let proposalId;
  await check('agent proposal appears: badge, toast, pending styling in the quilt', async () => {
    const r = await agent('/proposals', {
      summary: 'Add Zoe, Cy born 1975',
      source_text: 'Cy was born in 1975 and has a daughter Zoe.',
      people: [{ ref: 'z', given_name: 'Zoe', family_name: 'Root', sex: 'F', birth_date: '2005' }],
      relationships: [{ type: 'parent_child', a: cy.id, b: 'z', kind: 'birth' }],
      changes: [{ target_type: 'person', target_id: cy.id, op: 'update', fields: { birth_date: { from: null, to: '1975' } } }],
    });
    proposalId = r.proposal_id || r.id;
    assert.ok(proposalId);
    await page.waitForFunction(() => !document.getElementById('review-badge').hidden, null, { timeout: 12000 });
    assert.equal(await page.textContent('#review-badge'), '1');
    assert.match(await page.textContent('#toasts'), /new proposal/i);
    await page.waitForFunction(() => window.__app.view.q.n === 5, null, { timeout: 12000 });
    // Zoe is pending: her cell is amber-ish hatched, not plain black.
    await page.evaluate(() => {
      const v = window.__app.view;
      v.tf = { k: 1.5, x: 260, y: 70 };
      v.anim = null;
      v.dirty = true;
    });
    await page.waitForTimeout(300);
    const z = await where('Zoe');
    const flagged = await page.evaluate(() => window.__app.view.q.people.find((p) => p.n.startsWith('Zoe')).p);
    assert.equal(flagged, 1);
    // the link to Zoe is waiting for review: an amber ring with a slash instead of her mark
    const px = await pixel(z.x + 6.2 * 1.5, z.y);
    assert.ok(px[0] > px[2] + 40, `amber ring expected, got ${px}`);
    await page.screenshot({ path: path.join(out, 'light-pending.png') });
  });

  await check('review screen: your words, before/after, picture, approve', async () => {
    await page.click('#nav-review');
    await page.waitForSelector('#review-approve');
    const text = await page.textContent('#review-root');
    assert.match(text, /Cy was born in 1975/);
    assert.match(text, /1975/);
    assert.match(text, /Zoe/);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: path.join(out, 'light-review.png') });
    await page.click('#review-approve');
    await page.waitForFunction(() => document.getElementById('review-badge').hidden, null, { timeout: 8000 });
    const people = await ui('/people?q=Zoe');
    assert.equal(people.people.length, 1);
    const cyNow = await ui('/people/' + cy.id);
    assert.match(JSON.stringify(cyNow.person.birth), /1975/);
  });

  await check('a rejected proposal changes nothing', async () => {
    await agent('/proposals', { summary: 'Add Ghost', people: [{ ref: 'g', given_name: 'Ghost', family_name: 'Nobody', sex: 'M' }] });
    await page.click('#nav-review');
    await page.waitForSelector('#review-reject', { timeout: 12000 });
    await page.click('#review-reject');
    await page.click('dialog[open] .btn.danger');
    await page.waitForFunction(() => document.getElementById('review-badge').hidden, null, { timeout: 8000 });
    const found = await ui('/people?q=Ghost');
    assert.equal(found.people.length, 0);
  });

  await check('review: correct a value, reject with a reason; the assistant can read both', async () => {
    const r = await agent('/proposals', { summary: 'Add Tala', people: [{ ref: 't', given_name: 'Tala', family_name: 'Rahimi', sex: 'F', birth_date: '1980' }] });
    await page.click('#nav-review');
    await page.waitForSelector('.item .btn[aria-label="Correct a value"]', { timeout: 12000 });
    await page.waitForTimeout(1500); // the panel may still re-draw itself once after the proposal arrives
    await page.click('.item .btn[aria-label="Correct a value"]');
    await page.waitForSelector('.edit-form:not([hidden])', { timeout: 5000 });
    await page.screenshot({ path: path.join(out, 'light-review-edit.png') });
    allow400 = true;
    await page.fill('.edit-form input[id$="-birth_date"]', '19x1', { timeout: 4000 }).catch(async (e) => { throw new Error(e.message.split('\n')[0] + ' | ' + (await page.innerHTML('.edit-form')).slice(0, 1200)); });
    await page.click('.edit-form .btn.primary');
    await page.waitForFunction(() => /birth date/i.test(document.querySelector('.edit-form .conflict:not([hidden])')?.textContent || ''));
    allow400 = false;
    await page.fill('.edit-form input[id^="en-"]', 'checked her certificate');
    await page.fill('.edit-form input[id$="-birth_date"]', '1981').catch(async (e) => { throw new Error(e.message.split('\n')[0] + ' | ' + (await page.innerHTML('#review-root')).slice(0, 1500)); });
    await page.click('.edit-form .btn.primary');
    await page.waitForSelector('.item .chip', { timeout: 8000 });
    await page.click('#review-approve');
    await page.waitForFunction(() => document.getElementById('review-badge').hidden, null, { timeout: 8000 });
    const tala = (await ui('/people?q=Tala')).people[0];
    assert.match(JSON.stringify((await ui('/people/' + tala.id)).person.birth), /1981/);
    const seen = await agent('/people/' + tala.id);
    const corrected = seen.history.find((e) => e.event === 'corrected');
    assert.deepEqual(corrected.changes[0], { field: 'birth_date', from: '1980', to: '1981' });
    assert.equal(corrected.note, 'checked her certificate');
    assert.equal(seen.user_confirmed.birth_date.value, '1981');
    assert.ok(seen.history.some((e) => e.event === 'approved'));

    // Rejecting with a reason: the next agent that searches or resends is told.
    const bad = { summary: 'Add Bogus', people: [{ ref: 'b', given_name: 'Bogus', family_name: 'Person', sex: 'M', birth_date: '1900' }] };
    await agent('/proposals', bad);
    await page.click('#nav-review');
    await page.waitForSelector('#review-reject', { timeout: 12000 });
    await page.click('#review-reject');
    await page.click('#dlg-reject [data-reason="not_wanted"]');
    await page.screenshot({ path: path.join(out, 'light-reject.png') });
    await page.fill('#reject-note', 'made up');
    await page.click('#dlg-reject .btn.danger');
    await page.waitForFunction(() => document.getElementById('review-badge').hidden, null, { timeout: 8000 });
    const found = await agent('/people?q=Bogus');
    assert.equal(found.people.length, 0);
    assert.equal(found.previously_rejected[0].note, 'not_wanted: made up');
    await assert.rejects(() => agent('/proposals', bad), /previously_rejected/);
    const mine = await agent('/proposals');
    assert.ok(JSON.stringify(mine).includes('not_wanted: made up'));
  });

  await check('review: a new person can be left out of the approval, and their link goes with them', async () => {
    await agent('/proposals', {
      summary: 'Add Ivy and Jon',
      people: [{ ref: 'i', given_name: 'Ivy', family_name: 'Leftin', sex: 'F' }, { ref: 'j', given_name: 'Jon', family_name: 'Leftout', sex: 'M' }],
      relationships: [{ type: 'parent_child', a: anna.id, b: 'i', kind: 'birth' }, { type: 'parent_child', a: anna.id, b: 'j', kind: 'birth' }],
    });
    await page.click('#nav-review');
    await page.waitForSelector('#review-approve', { timeout: 12000 });
    const rows = page.locator('.item-row', { hasText: 'Jon' });
    await rows.first().locator('button[aria-pressed]').click();
    // his link is dimmed with him and cannot be toggled on its own
    assert.equal(await page.locator('.item-row.out').count(), 2);
    assert.equal(await page.locator('.item-row.out button[aria-pressed][disabled]').count(), 1);
    assert.match(await page.textContent('#review-approve'), /leaving out 2/i);
    await page.click('#review-approve');
    await page.waitForFunction(() => document.getElementById('review-badge').hidden, null, { timeout: 8000 });
    assert.equal((await ui('/people?q=Ivy')).people.length, 1);
    assert.equal((await ui('/people?q=Jon')).people.length, 0);
    await ui('/people/' + (await ui('/people?q=Ivy')).people[0].id, 'DELETE');
    await page.click('#nav-quilt');
  });

  await check('security: set a password, lock, wrong then right password, forgot it and use the recovery key, remove it', async () => {
    assert.equal(await page.locator('#nav-lock').isHidden(), true); // nothing to lock without a password
    await page.waitForTimeout(1500); // let the screen finish re-drawing after the earlier approvals
    await page.click('#settings');
    await page.click('#set-add-pw');
    await page.fill('#pw-new', 'first password');
    await page.fill('#pw-new2', 'first password');
    await page.click('#security-form .btn.primary');
    await page.waitForSelector('#gate-recovery');
    const rk = await page.textContent('#recovery-key');
    assert.match(rk, /^[A-Z2-7]{4}(-[A-Z2-7]{4})+$/);
    assert.equal(await page.locator('#gate-continue').isDisabled(), true);
    await page.check('#gate-kept');
    await page.click('#gate-continue');
    await page.waitForSelector('#gate-recovery', { state: 'detached' });
    assert.equal((await ui('/tree')).protected, true);
    // lock from the rail button: the unlock screen replaces the app and the data is refused
    assert.equal(await page.locator('#nav-lock').isVisible(), true);
    await page.click('#nav-lock');
    await page.waitForSelector('#gate-unlock-card', { timeout: 8000 }).catch(async (e) => {
      const info = await page.evaluate(() => `${document.querySelector('.gate') ? 'gate shown' : 'no gate'} dialogs=${document.querySelectorAll('dialog[open]').length} lockhidden=${document.getElementById('nav-lock').hidden}`);
      throw new Error(`${e.message} | page: ${info} tree=${JSON.stringify(await ui('/tree').catch((x) => x.message)).slice(0, 120)} ring=${ring.join(' // ')}`);
    });
    assert.equal((await fetch(base + '/api/people?q=Anna', { headers: { 'Sec-Fetch-Site': 'same-origin' } })).status, 423);
    assert.equal(await page.locator('.app').getAttribute('inert'), '');
    await page.fill('#gate-pw', 'not it');
    await page.click('#gate-unlock');
    await page.waitForFunction(() => /not right/.test(document.getElementById('gate-err').textContent));
    // forgot: the recovery key sets a new password
    await page.click('#gate-forgot');
    await page.fill('#gate-rk', rk);
    await page.fill('#gate-pw', 'second password');
    await page.fill('#gate-pw2', 'second password');
    await page.click('#gate-reset');
    await page.waitForSelector('html[data-ready="1"]');
    await page.waitForFunction(() => window.__app.view.q && window.__app.view.q.n >= 4);
    // lock again and unlock with the new password
    await ui('/tree/lock', 'POST', {});
    await page.waitForSelector('#gate-unlock-card', { timeout: 12000 }); // the status poll notices
    await page.fill('#gate-pw', 'second password');
    await page.click('#gate-unlock');
    await page.waitForSelector('html[data-ready="1"]');
    await page.waitForFunction(() => window.__app.view.q && window.__app.view.q.n >= 4);
    // remove the password again
    await page.click('#settings');
    await page.click('#set-remove-pw');
    await page.fill('#pw-current2', 'second password');
    await page.click('#security-form .btn.primary');
    await page.waitForFunction(() => document.getElementById('set-add-pw'));
    assert.equal((await ui('/tree')).protected, false);
    assert.equal(await page.locator('#nav-lock').isHidden(), true);
    await page.keyboard.press('Escape');
  });

  await check('closing the window asks only while a dialog is open', async () => {
    await page.evaluate(() => { window.__quit = []; window.ipc = { postMessage: (m) => window.__quit.push(m) }; });
    await page.evaluate(() => window.fqCloseRequested());
    assert.deepEqual(await page.evaluate(() => window.__quit), ['quit']); // nothing open: closes at once
    await page.click('#settings');
    await page.waitForFunction(() => document.getElementById('dlg-settings').open);
    await page.evaluate(() => { window.__quit = []; });
    await page.evaluate(() => { window.fqCloseRequested().catch((e) => (window.__qerr = String(e))); });
    await page.waitForFunction(() => document.querySelectorAll('dialog[open]').length === 2).catch(async (e) => { throw new Error(e.message + ' | ' + (await page.evaluate(() => window.__qerr + ' ' + [...document.querySelectorAll('dialog')].map((d) => d.id + ':' + d.open).join(',')))); });
    await page.click('dialog[open]:last-of-type .actions .btn:not(.primary)'); // Cancel
    assert.deepEqual(await page.evaluate(() => window.__quit), []);
    await page.evaluate(() => { window.fqCloseRequested(); });
    await page.waitForFunction(() => document.querySelectorAll('dialog[open]').length === 2);
    await page.click('dialog[open]:last-of-type .actions .btn.primary'); // Close anyway
    await page.waitForFunction(() => window.__quit.length === 1);
    await page.keyboard.press('Escape');
  });

  await check('keyboard: arrows pan, + and - zoom, 0 fits; toolbar targets are at least 44 px', async () => {
    await page.click('#nav-quilt');
    await page.focus('#quilt');
    const a = await page.evaluate(() => ({ ...window.__app.view.tf }));
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('+');
    const b = await page.evaluate(() => ({ ...window.__app.view.tf }));
    assert.ok(b.x !== a.x && b.k > a.k);
    await page.keyboard.press('0');
    await page.waitForTimeout(400);
    const small = await page.$$eval('.top .btn:not(.small), .rail-btn, .top input, .zoomctl .btn', (els) => els.filter((e) => e.offsetParent).map((e) => [e.id || e.textContent.trim(), e.getBoundingClientRect()]).filter(([, r]) => r.height < 43.5 || r.width < 43.5).map(([n]) => n));
    assert.deepEqual(small, []);
  });

  await check('highlight modes: relatives of a selection are tinted, "none" tints nothing, blood toggle changes reach', async () => {
    const r = await page.evaluate(() => {
      const v = window.__app.view;
      const q = v.q;
      const bob = q.people.findIndex((p) => p.n.startsWith('Bob'));
      v.setSelection([{ kind: 'row', ids: [q.people[bob].id] }]);
      v.setHighlight('all', true);
      const all = v.relRows.size;
      v.setHighlight('none', true);
      const none = v.relRows.size;
      v.setHighlight('descendants', true);
      const blood = v.relRows.size;
      v.setHighlight('descendants', false);
      const family = v.relRows.size;
      v.clearSelection();
      v.setHighlight('all', true);
      return { all, none, blood, family };
    });
    assert.ok(r.all >= 1);
    assert.equal(r.none, 0);
    assert.ok(r.family > r.blood, 'adopted Dee only counts when the whole family is followed: ' + JSON.stringify(r));
  });

  await check('label modes, layer-by-layer mode and privacy mode', async () => {
    await page.click('#view-btn');
    await page.selectOption('#label-mode', 'surname');
    assert.equal(await page.evaluate(() => window.__app.view.q.label(0, 'surname')), 'Root');
    await page.selectOption('#label-mode', 'full');
    await page.selectOption('#layout-mode', 'layer');
    await page.waitForFunction(() => window.__app.view.q.payload.mode === 'layer');
    await page.selectOption('#layout-mode', 'tree');
    await page.waitForFunction(() => window.__app.view.q.payload.mode === 'tree');
    await page.keyboard.press('Escape');
    await page.click('#settings');
    await page.check('#set-privacy');
    assert.equal(await page.evaluate(() => window.__app.view.privacy), true);
    await page.uncheck('#set-privacy');
    await page.keyboard.press('Escape');
  });

  await check('big file: quick layout first, progress strip, one glide at the end, selection and view stay put; keep-current cancels', async () => {
    const gen = process.env.GEN_GEDCOM || path.join(path.dirname(exe), 'gen_gedcom');
    const ged = await new Promise((resolve, reject) => {
      const c = spawn(gen, ['400', '5'], { stdio: ['ignore', 'pipe', 'inherit'] });
      let b = '';
      c.stdout.on('data', (d) => (b += d));
      c.on('close', () => resolve(b));
      c.on('error', reject);
    });
    const res = await fetch(base + '/api/import/gedcom', { method: 'POST', headers: { 'Sec-Fetch-Site': 'same-origin' }, body: ged });
    assert.ok(res.ok);
    // The built-in annealing engine is the slow one that shows the progress strip (the Graphviz layout is fast).
    await page.click('#nav-quilt');
    if (await page.locator('#engine-mode').isHidden()) await page.click('#view-btn');
    await page.selectOption('#engine-mode', 'own');
    if (await page.locator('#engine-mode').isVisible()) await page.click('#view-btn');
    await page.evaluate(() => window.__app.refreshAll());
    await page.waitForFunction(() => window.__app.view.q.n > 300);
    // The strip shows the real stage while the careful layout runs.
    await page.waitForFunction(() => !document.getElementById('strip').hidden, null, { timeout: 15000 });
    assert.match(await page.textContent('#strip-text'), /Ranking|Ordering|Polishing|Improving/);
    // Select someone and look at the view: neither may change when the better layout arrives.
    const id = await page.evaluate(() => {
      const v = window.__app.view;
      v.tf = { k: 0.6, x: 300, y: 80 };
      v.anim = null;
      const p = v.q.people[5];
      v.setSelection([{ kind: 'row', ids: [p.id] }]);
      return p.id;
    });
    const tf0 = await page.evaluate(() => ({ ...window.__app.view.tf }));
    await page.waitForFunction(() => document.getElementById('strip').hidden, null, { timeout: 120000 });
    await page.waitForFunction(() => !window.__app.view.anim, null, { timeout: 5000 });
    const after = await page.evaluate(() => ({ tf: { ...window.__app.view.tf }, sel: window.__app.view.sel.map((s) => s.ids[0]), layout: window.__app.view.q.payload.layout }));
    assert.deepEqual(after.tf, tf0);
    assert.deepEqual(after.sel, [id]);
    assert.equal(after.layout, 'full');
    // Start another careful layout and keep the current one.
    await page.click('#view-btn');
    await page.selectOption('#layout-mode', 'layer');
    await page.waitForFunction(() => !document.getElementById('strip').hidden, null, { timeout: 15000 });
    await page.click('#strip-keep');
    await page.waitForFunction(() => document.getElementById('strip').hidden);
    let pr;
    for (let i = 0; i < 100; i++) {
      pr = await ui('/quilt/progress?mode=layer&pending=1');
      if (pr.job.state !== 'running') break;
      await sleep(100);
    }
    assert.ok(pr.job.state === 'cancelled' || pr.job.state === 'done', 'the job stops soon after "keep current layout": ' + JSON.stringify(pr.job));
    await page.click('#view-btn');
    await page.selectOption('#layout-mode', 'tree');
    await page.waitForFunction(() => window.__app.view.q.payload.mode === 'tree');
    await page.keyboard.press('Escape');
  });

  await check('Graphviz engine is optional: used when found, the built-in order otherwise, and the geometry rules still hold', async () => {
    const hasDot = spawnSync('dot', ['-V']).status === 0;
    if (await page.locator('#engine-mode').isHidden()) await page.click('#view-btn');
    await page.selectOption('#engine-mode', 'dot');
    await page.waitForFunction(() => document.getElementById('strip').hidden && window.__app.view.q.payload.layout === 'full' && window.__app.view.q.payload.engine !== 'port' && !window.__app.view.anim, null, { timeout: 30000 }).catch(async (e) => { throw new Error(e.message + ' LOG ' + srvLog.slice(-800) + ' state ' + JSON.stringify(await page.evaluate(() => ({ strip: document.getElementById('strip').hidden, layout: window.__app.view.q.payload.layout, engine: window.__app.view.q.payload.engine, anim: !!window.__app.view.anim, job: window.__app.view.q.payload.job })))); });
    const r = await page.evaluate(() => ({ engine: window.__app.view.q.payload.engine, note: document.getElementById('engine-note').textContent, n: window.__app.view.q.n }));
    if (hasDot) assert.equal(r.engine, 'dot');
    else assert.ok(r.engine.startsWith('own'), r.engine);
    assert.ok(r.note.length > 0, 'the View menu says which engine made the order');
    const bad = await page.evaluate(() => {
      const v = window.__app.view; const q = v.q; const g = v.geo; const out = [];
      q.cols.forEach((col, j) => {
        for (const x of col.p) if (!(g.chy[j] >= g.ry[x] + 20)) out.push('header above parent ' + x);
        for (const c of col.c) if (!(g.chy[j] + 20 <= g.ry[c.r])) out.push('header below child ' + c.r);
      });
      return out;
    });
    assert.deepEqual(bad, []);
    await page.selectOption('#engine-mode', 'own');
    await page.waitForFunction(() => window.__app.view.q.payload.engine === 'own');
    await page.keyboard.press('Escape');
  });

  await check('Fit shows the whole quilt with a margin on every side, bottom row included', async () => {
    await page.keyboard.press('Escape');
    await page.click('#fit');
    await page.waitForFunction(() => !window.__app.view.tween, null, { timeout: 5000 });
    const r = await page.evaluate(() => { const v = window.__app.view; const g = v.geo; return { x0: v.tf.x, y0: v.tf.y, x1: v.tf.x + g.width * v.tf.k, y1: v.tf.y + g.height * v.tf.k, w: v.size.w, h: v.size.h }; });
    for (const e of [r.x0, r.y0, r.w - r.x1, r.h - r.y1]) assert.ok(e >= 20, 'margin ' + e + ' ' + JSON.stringify(r));
  });

  await check('level of detail: far out the quilt is grey blocks or a faded grid, close up everything', async () => {
    const r = await page.evaluate(() => { const v = window.__app.view; const out = {}; for (const k of [0.01, 0.05, 0.5]) { v.zoomAt(k / v.tf.k); out[k] = v.lod(); v.draw(); } v.fit(false); return out; });
    assert.deepEqual(r, { 0.01: 'blocks', 0.05: 'faded', 0.5: 'full' });
  });

  await check('on a large file the Graphviz layout is not started on its own, but can be asked for', async () => {
    if (await page.locator('#engine-mode').isHidden()) await page.click('#view-btn');
    await page.selectOption('#engine-mode', 'port');
    await page.waitForFunction(() => document.getElementById('strip').hidden && window.__app.view.q.payload.layout === 'full' && !window.__app.view.anim, null, { timeout: 60000 });
    const r = await page.evaluate(() => ({ engine: window.__app.view.q.payload.engine, n: window.__app.view.q.n, btn: !document.getElementById('engine-force').hidden, note: document.getElementById('engine-note').textContent }));
    assert.ok(r.n > 300, 'the test needs more people than the cutoff: ' + r.n);
    assert.ok(r.engine.startsWith('own (large file'), r.engine);
    assert.ok(r.btn && r.note.includes('Large file'), 'the View menu offers the Graphviz layout');
    await page.click('#engine-force');
    await page.waitForFunction(() => document.getElementById('strip').hidden && window.__app.view.q.payload.layout === 'full' && window.__app.view.q.payload.engine === 'port' && !window.__app.view.anim, null, { timeout: 120000 });
    assert.ok(await page.locator('#engine-force').isHidden());
    await page.selectOption('#engine-mode', 'own');
    await page.waitForFunction(() => window.__app.view.q.payload.engine === 'own');
    await page.keyboard.press('Escape');
  });

  await check('the built-in Graphviz layout needs no program, and the geometry rules still hold', async () => {
    if (await page.locator('#engine-mode').isHidden()) await page.click('#view-btn');
    await page.selectOption('#engine-mode', 'port');
    await page.waitForFunction(() => !document.getElementById('engine-force').hidden, null, { timeout: 30000 }); // the file is above the test's cutoff
    await page.click('#engine-force');
    await page.waitForFunction(() => document.getElementById('strip').hidden && window.__app.view.q.payload.layout === 'full' && window.__app.view.q.payload.engine === 'port' && !window.__app.view.anim, null, { timeout: 60000 });
    const r = await page.evaluate(() => ({ engine: window.__app.view.q.payload.engine, note: document.getElementById('engine-note').textContent, n: window.__app.view.q.n }));
    assert.equal(r.engine, 'port');
    assert.ok(r.note.length > 0, 'the View menu says which engine made the order');
    const bad = await page.evaluate(() => {
      const v = window.__app.view; const q = v.q; const g = v.geo; const out = [];
      q.cols.forEach((col, j) => {
        for (const x of col.p) if (!(g.chy[j] >= g.ry[x] + 20)) out.push('header above parent ' + x);
        for (const c of col.c) if (!(g.chy[j] + 20 <= g.ry[c.r])) out.push('header below child ' + c.r);
      });
      return out;
    });
    assert.deepEqual(bad, []);
    await page.selectOption('#engine-mode', 'own');
    await page.waitForFunction(() => window.__app.view.q.payload.engine === 'own');
    await page.keyboard.press('Escape');
  });

  await check('dark theme renders and differs from light', async () => {
    await page.click('#nav-quilt');
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.evaluate(() => window.__app.view.readColors());
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(out, 'dark-quilt.png') });
    const bg = await page.evaluate(() => window.__app.view.colors.bg);
    assert.notEqual(bg.toLowerCase(), '#ffffff');
  });

  await check('settings, export and agent details open', async () => {
    await page.click('#settings');
    await page.waitForSelector('#dlg-settings[open]');
    await page.click('#agent-show');
    await page.waitForSelector('#agent-test');
    await page.click('#agent-test');
    await page.waitForFunction(() => /works/.test(document.getElementById('toasts').textContent));
    await page.keyboard.press('Escape');
    await page.click('#export');
    await page.waitForSelector('#dlg-export[open]');
    const [dl] = await Promise.all([page.waitForEvent('download'), page.click('#exp-json')]);
    assert.match(dl.suggestedFilename(), /\.json$/);
    const [dl2] = await Promise.all([page.waitForEvent('download'), (async () => { await page.click('#export'); await page.click('#exp-svg'); })()]);
    assert.match(dl2.suggestedFilename(), /\.svg$/);
  });

  await check('calendar setting: stored Gregorian dates read in Jalali and back', async () => {
    const p3 = await ctx.newPage();
    await p3.goto(base + '/');
    await p3.waitForSelector('html[data-ready="1"]');
    const read = (cal) => p3.evaluate(async (c) => {
      localStorage.setItem('fq.calendar', c);
      const m = await import('/js/i18n.js');
      return [m.fmtDate('1950-08-03', 'exact'), m.fmtDate('1950', 'about'), m.fmtDate('1950-08', 'exact')];
    }, cal);
    const g = await read('gregorian');
    assert.match(g[0], /1950/);
    const j = await read('jalali');
    assert.match(j[0], /1329/, j[0]);
    assert.match(j[0], /12/, j[0]);
    assert.match(j[1], /1329/, j[1]);
    assert.match(j[2], /1329/, j[2]);
    await read('gregorian');
    await p3.close();
  });

  await check('calendar setting: a date typed in Jalali is stored as Gregorian', async () => {
    const p4 = await ctx.newPage();
    await p4.goto(base + '/');
    await p4.waitForSelector('html[data-ready="1"]');
    await p4.evaluate(async () => {
      localStorage.setItem('fq.calendar', 'jalali');
      (await import('/js/sidebar.js')).addFirstPerson();
    });
    await p4.fill('#pf-given', 'Jalali');
    await p4.fill('#pf-family', 'Typed');
    await p4.fill('#pf-bd', '1329-05-12');
    await p4.click('#pf-save');
    await p4.waitForSelector('#dlg-person', { state: 'hidden' });
    const found = await ui('/people?q=Jalali%20Typed');
    const hit = (found.people || found)[0];
    assert.ok(hit, JSON.stringify(found));
    const full = await ui('/people/' + hit.id);
    assert.equal(String((full.person || full).birth.date), '1950-08-03', JSON.stringify(full));
    await p4.evaluate(() => localStorage.setItem('fq.calendar', 'gregorian'));
    await p4.close();
  });

  await check('no missing translation keys and no console errors', async () => {
    assert.deepEqual(warnings, []);
    assert.deepEqual(errors, []);
  });

  await check('pseudo-language shows every string through the table', async () => {
    const p2 = await ctx.newPage();
    await p2.goto(base + '/?lang=qps');
    await p2.waitForSelector('html[data-ready="1"]');
    const txt = await p2.textContent('#legend');
    assert.match(txt, /⟦/);
    const plain = await p2.$$eval('.top button, .zoomctl button, .zoomctl option', (els) => els.map((e) => e.textContent.trim()).filter((x) => x && !x.includes('⟦') && !/^[A-Za-z0-9]{0,2}$|^[−+↑↓“” ]*$/.test(x)));
    assert.deepEqual(plain, [], 'untranslated toolbar strings: ' + plain.join(', '));
  });
} catch (e) {
  failures++;
  results.push('FATAL ' + (e.stack || e));
} finally {
  await browser?.close();
  srv.kill();
  console.log(results.join('\n'));
  console.log(failures ? `\n${failures} failed` : '\nall passed');
  process.exit(failures ? 1 : 0);
}
