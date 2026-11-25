// Typing dates in the calendar chosen in Settings. The tree stores Gregorian; the server converts what is typed.
import { api } from './api.js';
import { pref } from './prefs.js';

export const DATE_FIELDS = ['birth_date', 'death_date', 'start_date', 'end_date'];
const jalali = () => pref('calendar') === 'jalali';

/** A stored Gregorian date as the text to show in an input: the same date in the chosen calendar, YYYY[-MM[-DD]] with Latin digits. */
export function toInput(stored) {
  const s = stored === null || stored === undefined ? '' : String(stored);
  const m = /^(\d{4})(?:-(\d\d))?(?:-(\d\d))?$/.exec(s);
  if (!m || !jalali()) return s;
  // Same choice as fmtDate: a year or month is read at its middle.
  const d = new Date(Date.UTC(+m[1], m[2] ? +m[2] - 1 : 6, m[3] ? +m[3] : m[2] ? 15 : 1));
  const parts = Object.fromEntries(new Intl.DateTimeFormat('en-u-ca-persian-nu-latn', { year: 'numeric', month: '2-digit', day: '2-digit', timeZone: 'UTC' }).formatToParts(d).map((p) => [p.type, p.value]));
  const y = String(parts.relatedYear || parts.year).padStart(4, '0');
  if (m[3]) return `${y}-${parts.month}-${parts.day}`;
  return m[2] ? `${y}-${parts.month}` : y;
}

/** Text typed in the input -> `{stored, approximate, note}` (Gregorian). Throws the server's message for a bad date. */
export async function fromInput(text) {
  const date = String(text ?? '').trim();
  if (!date || !jalali()) return { stored: date, approximate: false };
  return api('/calendar/convert', { method: 'POST', body: { calendar: 'jalali', date } });
}
