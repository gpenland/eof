// Natural-language quick add: "Quiz 4 fri #cs2110 !" → { title: "Quiz 4", due: Fri, tags: [cs2110], starred }
// Every recognized token becomes a chip in the composer; clicking its × keeps the words literal.

import { addDays, iso, today } from './util.js';

const WD = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
const WD_RE = '(sun(?:day)?|mon(?:day)?|tue(?:s|sday)?|wed(?:nesday)?|thu(?:r|rs|rsday)?|fri(?:day)?|sat(?:urday)?)';
const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
const MON_RE = '(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t|tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)';

const nextWeekday = (from, wd, skipThisWeek = false) => {
  let d = addDays(from, 0);
  while (d.getDay() !== wd) d = addDays(d, 1);
  if (skipThisWeek) d = addDays(d, 7);
  return d;
};

// Each rule: regex (anchored on word boundaries) → { kind, value, label }
const RULES = [
  { re: /\bevery\s+weekday\b|\bweekdays\b/i, f: () => ({ kind: 'repeat', value: 'weekdays' }) },
  { re: /\bevery\s+day\b|\bdaily\b/i, f: () => ({ kind: 'repeat', value: 'daily' }) },
  { re: new RegExp(`\\bevery\\s+${WD_RE}\\b`, 'i'), f: (m, t) => ({ kind: 'repeat', value: 'weekly', due: nextWeekday(t, WD.indexOf(m[1].slice(0, 3).toLowerCase())) }) },
  { re: /\bevery\s+week\b|\bweekly\b/i, f: () => ({ kind: 'repeat', value: 'weekly' }) },
  { re: /\bevery\s+month\b|\bmonthly\b/i, f: () => ({ kind: 'repeat', value: 'monthly' }) },
  { re: /\bevery\s+year\b|\byearly\b|\bannually\b/i, f: () => ({ kind: 'repeat', value: 'yearly' }) },
  { re: /\b(today|tonight|tod)\b/i, f: (m, t) => ({ kind: 'due', due: t }) },
  { re: /\b(tomorrow|tmrw?|tmr)\b/i, f: (m, t) => ({ kind: 'due', due: addDays(t, 1) }) },
  { re: /\bnext\s+week\b/i, f: (m, t) => ({ kind: 'due', due: nextWeekday(addDays(t, 1), 1) }) },
  { re: /\b(this\s+)?weekend\b/i, f: (m, t) => ({ kind: 'due', due: nextWeekday(t, 6) }) },
  { re: /\bin\s+(\d{1,3}|a|an|one|two|three)\s+(day|week|month)s?\b/i, f: (m, t) => {
      const n = { a: 1, an: 1, one: 1, two: 2, three: 3 }[m[1].toLowerCase()] ?? Number(m[1]);
      const u = m[2].toLowerCase();
      if (u === 'month') { const d = new Date(t); d.setMonth(d.getMonth() + n); return { kind: 'due', due: d }; }
      return { kind: 'due', due: addDays(t, u === 'week' ? 7 * n : n) };
    } },
  { re: new RegExp(`\\b(next\\s+)?${WD_RE}\\b`, 'i'), f: (m, t) => ({ kind: 'due', due: nextWeekday(t, WD.indexOf(m[2].slice(0, 3).toLowerCase()), !!m[1]) }) },
  { re: new RegExp(`\\b${MON_RE}\\.?\\s+(\\d{1,2})(?:st|nd|rd|th)?(?:,?\\s+(\\d{4}))?\\b`, 'i'), f: (m, t) => monthDay(t, MONTHS.indexOf(m[1].slice(0, 3).toLowerCase()), +m[2], m[3]) },
  { re: new RegExp(`\\b(\\d{1,2})(?:st|nd|rd|th)?\\s+${MON_RE}\\b`, 'i'), f: (m, t) => monthDay(t, MONTHS.indexOf(m[2].slice(0, 3).toLowerCase()), +m[1]) },
  { re: /\b(\d{4})-(\d{2})-(\d{2})\b/, f: (m) => ({ kind: 'due', due: new Date(+m[1], +m[2] - 1, +m[3]) }) },
  { re: /\b(\d{1,2})\/(\d{1,2})(?:\/(\d{2,4}))?\b/, f: (m, t) => monthDay(t, +m[1] - 1, +m[2], m[3] && (m[3].length === 2 ? '20' + m[3] : m[3])) },
];

function monthDay(t, mon, day, year) {
  if (mon < 0 || mon > 11 || day < 1 || day > 31) return null;
  let d = new Date(year ? +year : t.getFullYear(), mon, day);
  if (d.getMonth() !== mon) return null;
  if (!year && d < t) d = new Date(t.getFullYear() + 1, mon, day); // "jan 5" in December means next year
  return { kind: 'due', due: d };
}

/**
 * @param {string} text
 * @param {Set<string>} ignored  token texts the user chose to keep literal
 */
export function parse(text, ignored = new Set()) {
  const t = today();
  let rest = ` ${text} `;
  const out = { title: '', due: null, repeat: null, tags: [], starred: false, chips: [] };

  // Tags: #word
  rest = rest.replace(/(^|\s)#([\p{L}\p{N}_-]+)/gu, (all, sp, tag) => {
    if (ignored.has('#' + tag)) return all;
    const tg = tag.toLowerCase();
    if (!out.tags.includes(tg)) out.tags.push(tg);
    out.chips.push({ kind: 'tag', label: '#' + tg, token: '#' + tag });
    return sp;
  });
  // Star: a standalone "!" or "!!"
  rest = rest.replace(/(^|\s)!{1,3}(?=\s)/, (all, sp) => {
    if (ignored.has('!')) return all;
    out.starred = true;
    out.chips.push({ kind: 'star', label: 'Starred', token: '!' });
    return sp;
  });
  for (const rule of RULES) {
    const m = rest.match(rule.re);
    if (!m || ignored.has(m[0].toLowerCase())) continue;
    const r = rule.f(m, t);
    if (!r) continue;
    if (r.kind === 'repeat') {
      if (out.repeat) continue;
      out.repeat = r.value;
      if (r.due && !out.due) out.due = r.due;
      out.chips.push({ kind: 'repeat', label: { daily: 'Every day', weekdays: 'Weekdays', weekly: 'Every week', monthly: 'Every month', yearly: 'Every year' }[r.value], token: m[0].toLowerCase() });
    } else {
      if (out.due) continue;
      out.due = r.due;
      out.chips.push({ kind: 'due', token: m[0].toLowerCase() });
    }
    rest = rest.replace(m[0], ' ');
  }
  if (out.repeat && !out.due) out.due = t;
  out.title = rest.replace(/\s+/g, ' ').trim();
  if (out.due) out.due = iso(out.due);
  return out;
}
