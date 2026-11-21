// Every text key the UI uses must exist in locales/en.json (static keys and the dynamic families).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const dir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const en = JSON.parse(fs.readFileSync(path.join(dir, 'locales/en.json'), 'utf8'));
const used = new Set();
for (const f of [...fs.readdirSync(path.join(dir, 'js')).map((x) => 'js/' + x), 'index.html']) {
  const s = fs.readFileSync(path.join(dir, f), 'utf8');
  for (const m of s.matchAll(/\bt\(\s*'([a-z_.0-9]+)'\s*[,)]/g)) used.add(m[1]);
  for (const m of s.matchAll(/data-i18n(?:-[a-z]+)?="([a-z_.0-9]+)"/g)) used.add(m[1]);
}
// Dynamic keys: t('prefix' + value). Each value that can occur must be in the table.
const dynamic = {
  'state.': ['married', 'unmarried', 'divorced', 'ended', 'unknown'], 'kind.': ['birth', 'adopted', 'foster', 'step', 'guardian', 'sealing', 'unknown'],
  'q.': ['exact', 'about', 'before', 'after', 'estimated'], 'sex.': ['male', 'female', 'other', 'unknown'], 'hl.': ['all', 'none', 'ancestors', 'descendants'],
  'strip.': ['starting', 'ranking', 'ordering', 'polishing', 'done'], 'side.add_': ['parent', 'partner', 'child', 'sibling'], 'dlg.add_': ['parent', 'partner', 'child', 'sibling'],
  'date.': ['about', 'before', 'after', 'estimated'], 'field.': ['sex', 'is_living', 'birth_date', 'birth_qualifier', 'birth_place', 'death_date', 'death_qualifier', 'death_place', 'note', 'name_type', 'given', 'family', 'full_name', 'script', 'kind', 'subtype', 'start_date', 'end_date', 'place'],
};
const missing = [];
for (const k of used) if (k.endsWith('.') ) { if (!dynamic[k]) missing.push(k + '(no table of values)'); } else if (!(k in en)) missing.push(k);
for (const [pre, vals] of Object.entries(dynamic)) for (const v of vals) if (!((pre + v) in en)) missing.push(pre + v);
for (const v of ['married', 'unmarried', 'divorced', 'ended', 'unknown']) if (!(`state.${v}.long` in en)) missing.push(`state.${v}.long`);
if (missing.length) {
  console.error('Missing translation keys:\n  ' + missing.join('\n  '));
  process.exit(1);
}
console.log(`ok: ${used.size} keys used, ${Object.keys(en).length} in en.json`);
