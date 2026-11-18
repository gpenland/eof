// Countdown — "the horizon".
// Two readings of distance, layered:
//  · a shared, linear time ruler: every event gets a reach bar on the same scale (today → the
//    farthest date), with month ticks, so "twice as far" literally looks twice as long;
//  · a gentle size/opacity falloff (log time) so near things still loom larger.
// Vertical spacing between rows also follows the gap in days. Past dates drop into the rear-view.
// A date can carry a T-minus plan: milestone tasks ("book flights 3 weeks before") that sit on its
// reach bar as notches and move with it when the date changes.

import { h, icon, get, post, patch, del, toast, fail, sheet, iso, parseISO, addDays, today, fmtShort, fmtLong, fmtTime, ago, fill, milestoneLabel } from './util.js';

let S = null;
const HORIZON_DAYS = 730; // things two years out are at the vanishing point

export function mount(root) {
  S = { root, items: [], timer: null, body: h('div') };
  root.append(h('div.horizon',
    h('div.page-head',
      h('div', h('div.eyebrow', 'Countdown'), h('h1', 'On the horizon')),
      h('div.grow'),
      h('button.btn.primary', { onclick: () => edit(null) }, icon('plus'), 'New date')),
    S.body));
  load();
  S.timer = setInterval(tick, 1000);
  return {
    unmount() { clearInterval(S.timer); S = null; },
    onChange(ev) { if (['countdowns', 'tasks', '*'].includes(ev.area)) load(); },
    key(e) { if (e.key === 'n') { e.preventDefault(); edit(null); } },
    create: () => edit(null),
  };
}

async function load() {
  try {
    const items = await get('/api/countdowns?include_past=true');
    if (!S) return;
    S.items = items;
    render();
  } catch (e) { fail(e); }
}

/** Moment the countdown aims at: midnight for all-day dates, else the exact time. */
const target = (c) => (c.all_day ? parseISO(c.date) : new Date(c.at));
const daysLeft = (c) => Math.max(0, (target(c) - Date.now()) / 864e5);
// Log-time closeness in [0,1], then eased so a week away feels clearly bigger than a month away.
const proximity = (c) => {
  const raw = Math.min(1, Math.max(0, 1 - Math.log1p(daysLeft(c)) / Math.log1p(HORIZON_DAYS)));
  return Math.max(0.02, raw ** 1.7);
};
const isPast = (c) => c.days_until < 0 || (c.days_until === 0 && !c.all_day && Date.now() - target(c) > 6 * 36e5);

function render() {
  const upcoming = S.items.filter((c) => !isPast(c)).sort((a, b) => target(a) - target(b));
  const past = S.items.filter(isPast).sort((a, b) => target(b) - target(a)).slice(0, 12);
  S.lastRender = Date.now();

  if (!upcoming.length && !past.length) {
    fill(S.body, h('div.empty', h('div.art', '◌'), h('h3', 'Nothing on the horizon'),
      h('p', 'Add a date you’re looking forward to — or dreading. It will grow on this page as it gets closer.'),
      h('p', { style: { marginTop: '16px' } }, h('button.btn', { onclick: () => edit(null) }, icon('plus'), 'Add the first date'))));
    return;
  }
  const scale = timeScale(upcoming);
  fill(S.body,
    h('div.road',
      upcoming.length ? ruler(scale) : null,
      upcoming.map((c, i) => eventEl(c, scale, i ? daysLeft(c) - daysLeft(upcoming[i - 1]) : 0))),
    upcoming.length ? h('div.horizon-line', 'horizon') : null,
    past.length ? h('div.rearview', h('h4', 'In the rear-view'),
      past.map((c) => h('div.past-ev',
        h('span.grow', c.title),
        h('span', { style: { font: '12px var(--mono)' } }, agoDays(c)),
        h('button.btn.sm.ghost.icon-only', { 'aria-label': `Remove ${c.title}`, onclick: () => remove(c) }, icon('x'))))) : null,
  );
  tick();
}

function agoDays(c) {
  const n = -c.days_until;
  return n <= 0 ? 'today' : n === 1 ? 'yesterday' : n < 60 ? `${n} days ago` : fmtLong(target(c));
}

/** Linear scale shared by every row: today at 0, the end of the farthest event's month at 1. */
function timeScale(upcoming) {
  const now = new Date();
  const far = upcoming.length ? target(upcoming[upcoming.length - 1]) : now;
  const end = new Date(far.getFullYear(), far.getMonth() + 1, 1); // round out to the next month start
  const span = Math.max(14, (end - now) / 864e5);
  const months = [];
  for (let m = new Date(now.getFullYear(), now.getMonth() + 1, 1); m <= end; m = new Date(m.getFullYear(), m.getMonth() + 1, 1)) {
    months.push({ at: (m - now) / 864e5 / span, date: m });
  }
  return { span, months, x: (days) => Math.min(1, Math.max(0, days / span)) };
}

const pct = (f) => `${(f * 100).toFixed(2)}%`;
const monthName = (d) => new Intl.DateTimeFormat(undefined, { month: 'short', ...(d.getMonth() === 0 ? { year: '2-digit' } : {}) }).format(d);

function ruler(scale) {
  // Label every month while they fit; thin out on long spans.
  const every = Math.ceil(scale.months.length / 10);
  return h('div.ruler', { 'aria-hidden': 'true' },
    h('span.today', { style: { left: '0%' } }, 'Today'),
    scale.months.map((m, i) => h('span', {
      // Skip labels that would collide with "Today" or be thinned out.
      class: [m.date.getMonth() === 0 && 'yr', ((i + 1) % every || m.at < 0.07) && 'minor', m.at < 0.16 && 'near'].filter(Boolean).join(' '),
      style: { left: pct(m.at) },
    }, monthName(m.date))));
}

function eventEl(c, scale, gapDays) {
  const s = proximity(c);
  const d = c.days_until;
  const reach = scale.x(daysLeft(c));
  const when = parseISO(c.date);
  const dateLabel = when.getFullYear() === today().getFullYear() ? fmtShort(when) : fmtLong(when);

  let num, unit;
  if (d === 0) { num = 'Today'; unit = c.all_day ? '' : (Date.now() < target(c) ? 'until ' + fmtTime(target(c)) : 'now'); }
  else if (!c.all_day && target(c) - Date.now() < 48 * 36e5) { num = h('span.tick', { 'data-at': c.at }, ''); unit = 'hours left'; }
  else { num = String(d); unit = d === 1 ? 'day' : 'days'; }

  const el = h('div.event', {
    class: [d === 0 && 'today', d > 0 && d <= 3 && 'soon'].filter(Boolean).join(' '),
    style: { '--s': s.toFixed(3), '--gap': Math.min(1, gapDays / 60).toFixed(3) },
    tabIndex: 0, role: 'button',
    'aria-label': `${c.title}, ${d === 0 ? 'today' : `in ${d} day${d === 1 ? '' : 's'}`}, ${dateLabel}`,
    onclick: () => edit(c),
    onkeydown: (e) => e.key === 'Enter' && edit(c),
  },
  h('div.num', num, unit ? h('small', unit) : null),
  h('div.what',
    h('div.title', c.title),
    h('div.when',
      h('span', dateLabel + (c.all_day ? '' : ' · ' + fmtTime(target(c)))),
      d > 13 ? h('span', '· ' + spanLabel(d)) : null,
      d === 0 && !c.all_day && Date.now() < target(c) ? h('span.tick', { 'data-at': c.at }) : null,
      c.created_by !== 'you' ? h('span.by-ai', '✦ ', c.created_by) : null),
    c.note ? h('div.note', c.note) : null,
    planLine(c),
    h('div.reach', { title: `${Math.round(daysLeft(c))} of the ${Math.round(scale.span)} days shown` },
      scale.months.map((m) => h('b', { class: m.date.getMonth() === 0 ? 'yr' : null, style: { left: pct(m.at) } })),
      h('i', { style: { width: pct(reach) } }),
      (c.milestones || []).filter((m) => m.due).map((m) => {
        const away = (parseISO(m.due) - Date.now()) / 864e5;
        return h('u', {
          class: m.done ? 'done' : away < -1 ? 'late' : '',
          style: { left: pct(scale.x(Math.max(0, away))) },
          title: `${milestoneLabel(m.days_before)} · ${m.title}${m.done ? ' (done)' : ''}`,
        });
      }),
      h('em', { style: { left: pct(reach) } }))));
  return el;
}

/** "Plan 2/5 · next: Book flights, in 3 days" under a countdown that has milestones. */
function planLine(c) {
  const ms = c.milestones || [];
  if (!ms.length) return null;
  const done = ms.filter((m) => m.done).length;
  const next = ms.find((m) => !m.done);
  let when = '';
  if (next?.due) {
    const n = Math.round((parseISO(next.due) - today()) / 864e5);
    when = n < 0 ? `${-n}d overdue` : n === 0 ? 'today' : n === 1 ? 'tomorrow' : `in ${n} days`;
  }
  return h('div.plan-line', { class: next && when.endsWith('overdue') ? 'late' : '' },
    h('span.count', `Plan ${done}/${ms.length}`),
    next ? h('span', `next: ${next.title}${when ? ` · ${when}` : ''}`) : h('span', 'all done'));
}

/** "book flights 3 weeks before" → { title: 'book flights', days: 21 }. `days` is null when no offset was given. */
function parseStep(text) {
  const word = { a: 1, an: 1, one: 1, two: 2, three: 3, four: 4, five: 5, six: 6 };
  const unit = { day: 1, week: 7, month: 30 };
  let s = ` ${text} `;
  let days = null;
  let m;
  if ((m = s.match(/\bt\s*-\s*(\d{1,4})\b/i))) days = Number(m[1]);
  else if ((m = s.match(/\b(\d{1,4}|a|an|one|two|three|four|five|six)\s+(day|week|month)s?\s+(before|prior|ahead|early|after|later)\b/i))) {
    const n = (word[m[1].toLowerCase()] ?? Number(m[1])) * unit[m[2].toLowerCase()];
    days = /after|later/i.test(m[3]) ? -n : n;
  } else if ((m = s.match(/\b(?:the\s+)?(day|week|night)\s+(before|after)\b/i))) {
    const n = m[1].toLowerCase() === 'week' ? 7 : 1;
    days = m[2].toLowerCase() === 'before' ? n : -n;
  } else if ((m = s.match(/\b(on\s+the\s+day|day\s+of|same\s+day)\b/i))) days = 0;
  if (m) s = s.replace(m[0], ' ');
  return { title: s.replace(/[:–—-]\s*$/, '').replace(/^\s*[:–—-]/, '').replace(/\s+/g, ' ').trim(), days };
}

function spanLabel(d) {
  if (d < 60) return `${Math.floor(d / 7)} wk ${d % 7 ? `${d % 7} d` : ''}`.trim();
  const months = Math.round(d / 30.44);
  if (months < 12) return `≈ ${months} months`;
  const y = Math.floor(months / 12), m = months % 12;
  return `≈ ${y} yr${m ? ` ${m} mo` : ''}`;
}

function tick() {
  if (!S || document.hidden) return;
  for (const el of S.root.querySelectorAll('.tick')) {
    const ms = Math.max(0, new Date(el.dataset.at) - Date.now());
    const hh = Math.floor(ms / 36e5), mm = Math.floor((ms % 36e5) / 6e4), ss = Math.floor((ms % 6e4) / 1e3);
    el.textContent = `${hh}:${String(mm).padStart(2, '0')}:${String(ss).padStart(2, '0')}`;
  }
  // Sizes drift continuously; repaint once a minute (or when the day rolls over).
  if (Date.now() - S.lastRender > 60_000) load();
}

function edit(c) {
  sheet((box, close) => {
    const title = h('input.input', { value: c?.title ?? '', placeholder: 'What are you counting down to?', maxLength: 200 });
    const date = h('input.input', { type: 'date', value: c?.date ?? '', required: true });
    const time = h('input.input', { type: 'time', value: c?.time ?? '' });
    const note = h('input.input', { value: c?.note ?? '', placeholder: 'Optional note' });
    const preview = h('div.preview');
    const upd = () => {
      if (!date.value) { preview.textContent = ''; return; }
      const n = Math.round((parseISO(date.value) - today()) / 864e5);
      preview.textContent = n === 0 ? 'That’s today.' : n > 0 ? `In ${n} day${n === 1 ? '' : 's'} — ${fmtShort(parseISO(date.value))}` : `${-n} days ago`;
    };
    date.addEventListener('input', upd); upd();

    // ── T-minus plan ──
    // Saved countdowns change their plan immediately; a new one sends its staged steps on save.
    const steps = c ? [...(c.milestones || [])] : [];
    const planList = h('ul.plan');
    const stepIn = h('input.input', { placeholder: 'Add a step — “book flights 3 weeks before”', maxLength: 300, 'aria-label': 'New step' });
    const daysIn = h('input.input.days', { type: 'number', value: 7, min: -365, max: 3650, 'aria-label': 'Days before' });
    const stepHint = h('span.muted.step-hint');
    const dueFor = (n) => (date.value ? addDays(parseISO(date.value), -n) : null);
    const drawPlan = () => {
      steps.sort((a, b) => b.days_before - a.days_before);
      fill(planList, ...steps.map((m) => {
        const d = dueFor(m.days_before);
        return h('li', { class: m.done ? 'done' : '' },
          c && m.id ? h('button.tick', { 'aria-label': m.done ? `Reopen “${m.title}”` : `Complete “${m.title}”`, onclick: async () => {
            try { await post(`/api/tasks/${m.id}/${m.done ? 'uncomplete' : 'complete'}`); m.done = !m.done; drawPlan(); } catch (e) { fail(e); }
          } }, icon('check')) : h('span.tick.ghost'),
          h('span.tm', milestoneLabel(m.days_before)),
          h('span.what', m.title),
          h('span.on', d ? fmtShort(d) : ''),
          h('button.btn.sm.ghost.icon-only', { 'aria-label': `Remove “${m.title}”`, onclick: async () => {
            try { if (c && m.id) await del(`/api/tasks/${m.id}`); steps.splice(steps.indexOf(m), 1); drawPlan(); } catch (e) { fail(e); }
          } }, icon('x')));
      }));
      const d = dueFor(Number(daysIn.value) || 0);
      stepHint.textContent = d ? `${milestoneLabel(Number(daysIn.value) || 0)} → ${fmtShort(d)}` : '';
    };
    stepIn.addEventListener('input', () => { const p = parseStep(stepIn.value); if (p.days != null) daysIn.value = p.days; drawPlan(); });
    daysIn.addEventListener('input', drawPlan);
    date.addEventListener('input', drawPlan);
    const addStep = async () => {
      const p = parseStep(stepIn.value);
      if (!p.title) return stepIn.focus();
      const days_before = Math.round(Number(daysIn.value) || 0);
      try {
        if (c) {
          const t = await post(`/api/countdowns/${c.id}/milestones`, { title: p.title, days_before });
          steps.push({ id: t.id, title: t.title, days_before, due: t.due, done: false });
        } else {
          steps.push({ title: p.title, days_before });
        }
        stepIn.value = '';
        drawPlan();
        stepIn.focus();
      } catch (e) { fail(e); }
    };
    stepIn.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); addStep(); } });
    daysIn.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); addStep(); } });
    drawPlan();

    const save = async () => {
      if (!title.value.trim() || !date.value) { (title.value.trim() ? date : title).focus(); return; }
      const body = { title: title.value.trim(), date: date.value, time: time.value || null, note: note.value };
      if (!c && steps.length) body.milestones = steps.map((m) => ({ title: m.title, days_before: m.days_before }));
      try {
        if (c) await patch(`/api/countdowns/${c.id}`, body);
        else await post('/api/countdowns', body);
        close(); load();
      } catch (e) { fail(e); }
    };
    [title, date, time, note].forEach((i) => i.addEventListener('keydown', (e) => e.key === 'Enter' && save()));
    box.append(
      h('h2', c ? 'Edit date' : 'New date'),
      h('label.field', h('span', 'Title'), title),
      h('div.row2', h('label.field', h('span', 'Date'), date), h('label.field', h('span', 'Time (optional)'), time)),
      h('label.field', h('span', 'Note'), note),
      preview,
      h('div.field.plan-field',
        h('span', 'Plan ', h('small.muted', '· tasks that count down with it')),
        planList,
        h('div.step-add', stepIn, daysIn, h('span.muted.days-label', 'days before'),
          h('button.btn.sm', { onclick: addStep, 'aria-label': 'Add step' }, icon('plus'))),
        stepHint),
      h('div.actions',
        c ? h('button.btn.ghost.danger', { onclick: () => { close(); remove(c); } }, icon('trash'), 'Delete') : null,
        h('span.grow'),
        h('button.btn.ghost', { onclick: close }, 'Cancel'),
        h('button.btn.primary', { onclick: save }, c ? 'Save' : 'Add date')));
  });
}

async function remove(c) {
  try {
    await del(`/api/countdowns/${c.id}`);
    load();
    toast(`Removed “${c.title}”`, { action: 'Undo', onAction: async () => {
      await post('/api/countdowns', { title: c.title, date: c.date, time: c.time, note: c.note });
      load();
    } });
  } catch (e) { fail(e); }
}
