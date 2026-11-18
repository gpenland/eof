// Tasks — simple lists, with three additions:
//  • a natural-language composer ("Quiz 4 fri #cs2110 !") that shows what it understood as chips,
//  • a two-week day ribbon you can drop tasks onto to reschedule them,
//  • provenance: anything an AI added is marked ✦ with its name.

import {
  h, icon, get, post, patch, del, toast, fail, menu, sheet, debounce, autosize, store, $, $$,
  iso, parseISO, addDays, today, dayDiff, relDay, fmtShort, fmtLong, fmtWd, ago, fill, milestoneLabel,
} from './util.js';
import { ctx } from './app.js';
import { parse } from './quickadd.js';

const REPEAT_LABEL = { daily: 'Daily', weekdays: 'Weekdays', weekly: 'Weekly', monthly: 'Monthly', yearly: 'Yearly' };
let S = null;

export function mount(root) {
  const lists = ctx.boot.lists;
  let listId = store.get('list');
  if (!lists.some((l) => l.id === listId)) listId = lists[0]?.id;
  S = {
    root, listId, lists,
    sort: store.get('sort', 'date'),
    tag: null, day: null,
    open: [], done: [],
    showDone: false, openId: null, focusId: null,
    ignored: new Set(), deferred: false, dragging: false,
  };
  S.el = {
    head: h('div.page-head'),
    composer: buildComposer(),
    toolbar: h('div.toolbar'),
    ribbon: h('div.ribbon', { 'aria-label': 'Next two weeks' }),
    tagbar: h('div.tagbar'),
    list: h('div', { role: 'list' }),
    done: h('div'),
  };
  root.append(h('div.tasks', S.el.head, S.el.composer, S.el.ribbon, S.el.tagbar, S.el.list, S.el.done));
  renderHead();
  load();
  const onDoc = (e) => {
    if (S.openId && !e.target.closest('.task.open, .menu, .sheet-backdrop')) closeEditor();
  };
  document.addEventListener('mousedown', onDoc);
  return {
    unmount() { document.removeEventListener('mousedown', onDoc); S = null; },
    onChange(ev) { if (ev.area === 'tasks' || ev.area === '*') refresh(); },
    key: onKey,
    reveal,
  };
}

function refresh() {
  if (!S) return;
  if (S.openId || S.dragging) { S.deferred = true; return; }
  load();
}

async function load() {
  try {
    const [open, done, lists] = await Promise.all([
      get(`/api/tasks?status=open&list_id=${S.listId}`),
      get(`/api/tasks?status=completed&limit=60&list_id=${S.listId}`),
      get('/api/lists'),
    ]);
    if (!S) return;
    Object.assign(S, { open, done, lists });
    ctx.boot.lists = lists;
    renderHead();
    renderAll();
  } catch (e) { fail(e); }
}

// ── Header: list switcher, sort, more ──

function currentList() { return S.lists.find((l) => l.id === S.listId) || S.lists[0]; }

function renderHead() {
  const l = currentList();
  const switcher = h('button.list-switch', { 'aria-haspopup': 'menu', onclick: (e) => listMenu(e.currentTarget) }, l?.name ?? 'Tasks', icon('chevron'));
  const seg = h('div.seg', { role: 'tablist', 'aria-label': 'Sort' },
    ['date', 'custom'].map((k) => h('button', {
      class: S.sort === k ? 'on' : '', role: 'tab', 'aria-selected': S.sort === k,
      onclick: () => { S.sort = k; store.set('sort', k); S.day = null; renderAll(); renderHead(); },
    }, k === 'date' ? 'Date' : 'My order')));
  const more = h('button.btn.ghost.icon-only', { 'aria-label': 'More', onclick: (e) => menu(e.currentTarget, [
    { label: `Clear completed (${S.done.length})`, icon: 'trash', onClick: clearDone },
    { label: 'Keyboard shortcuts', icon: 'type', onClick: () => document.dispatchEvent(new KeyboardEvent('keydown', { key: '?' })) },
  ]) }, icon('dots'));
  fill(S.el.head, h('div', h('div.eyebrow', 'Tasks'), switcher), h('div.grow'), seg, more);
}

function listMenu(anchor) {
  menu(anchor, [
    { section: 'Lists' },
    ...S.lists.map((l) => ({ label: `${l.name}${l.open_count ? `  ·  ${l.open_count}` : ''}`, check: l.id === S.listId, onClick: () => switchList(l.id) })),
    'hr',
    { label: 'New list…', icon: 'plus', onClick: () => nameSheet('New list', '', async (name) => { const l = await post('/api/lists', { name }); S.listId = l.id; store.set('list', l.id); load(); }) },
    { label: 'Rename list…', icon: 'type', onClick: () => nameSheet('Rename list', currentList().name, async (name) => { await patch(`/api/lists/${S.listId}`, { name }); load(); }) },
    S.lists.length > 1 ? { label: 'Delete list…', icon: 'trash', danger: true, onClick: deleteList } : null,
  ].filter(Boolean));
}

function switchList(id) {
  S.listId = id; store.set('list', id); S.tag = null; S.day = null; S.openId = null;
  load();
}

function nameSheet(title, value, save) {
  sheet((box, close) => {
    const input = h('input.input', { value, placeholder: 'Name', maxLength: 120 });
    const go = async () => { if (!input.value.trim()) return; try { await save(input.value.trim()); close(); } catch (e) { fail(e); } };
    input.addEventListener('keydown', (e) => e.key === 'Enter' && go());
    box.append(h('h2', title), input, h('div.actions', h('button.btn.ghost', { onclick: close }, 'Cancel'), h('button.btn.primary', { onclick: go }, 'Save')));
  });
}

function deleteList() {
  const l = currentList();
  sheet((box, close) => box.append(
    h('h2', `Delete “${l.name}”?`),
    h('p.muted', { style: { margin: 0 } }, `This deletes the list and its ${l.open_count} open task${l.open_count === 1 ? '' : 's'}. This can't be undone.`),
    h('div.actions', h('button.btn.ghost', { onclick: close }, 'Cancel'),
      h('button.btn.primary', { style: { background: 'var(--late)', borderColor: 'var(--late)' }, onclick: async () => {
        try { await del(`/api/lists/${l.id}`); close(); S.listId = null; store.set('list', null); ctx.boot.lists = await get('/api/lists'); S.lists = ctx.boot.lists; S.listId = S.lists[0].id; load(); } catch (e) { fail(e); }
      } }, 'Delete list'))));
}

async function clearDone() {
  if (!S.done.length) return;
  const n = S.done.length;
  try { await post(`/api/lists/${S.listId}/clear-completed`); toast(`Cleared ${n} completed`); load(); } catch (e) { fail(e); }
}

// ── Composer ──

function buildComposer() {
  const input = h('input', { placeholder: 'Add a task', 'aria-label': 'Add a task', autocomplete: 'off', enterKeyHint: 'done' });
  const chips = h('div.chips', { 'aria-live': 'polite' });
  const box = h('div.composer', h('div.row', icon('plus', 'plus'), input), chips);
  const hints = ['Add a task', 'Try “Quiz 4 fri #cs2110”', 'Try “Notebook every weekday !”', 'Try “Apply UNG mar 17 2027”'];
  let hi = 0;
  input.addEventListener('focus', () => { input.placeholder = hints[(hi = (hi % (hints.length - 1)) + 1)]; });
  input.addEventListener('blur', () => { input.placeholder = hints[0]; });

  const update = () => {
    if (!input.value.trim()) S.ignored.clear();
    const p = parse(input.value, S.ignored);
    fill(chips, ...p.chips.map((c) => h('span.pchip', { class: c.kind },
      icon({ due: 'cal', repeat: 'repeat', tag: 'tag', star: 'star' }[c.kind]),
      c.kind === 'due' ? relDay(p.due) : c.label,
      h('button', { 'aria-label': `Don't treat “${c.token}” as ${c.kind}`, title: 'Keep as text', onmousedown: (e) => e.preventDefault(),
        onclick: () => { S.ignored.add(c.token); update(); input.focus(); } }, icon('x')))));
    return p;
  };
  input.addEventListener('input', update);
  input.addEventListener('keydown', async (e) => {
    if (e.key === 'Escape') { input.value = ''; update(); input.blur(); return; }
    if (e.key !== 'Enter' || e.isComposing) return;
    const p = update();
    if (!p.title) return;
    input.value = ''; S.ignored.clear(); update();
    // In "date" view with a day selected in the ribbon, new tasks land on that day.
    const due = p.due ?? (S.day || null);
    try {
      const t = await post('/api/tasks', { list_id: S.listId, title: p.title, due, tags: p.tags, starred: p.starred, repeat: p.repeat });
      S.open.unshift(t);
      renderAll();
      flash(t.id);
    } catch (err) { fail(err); }
  });
  box.focusInput = () => input.focus();
  return box;
}

export async function quickAdd(text) {
  const p = parse(text);
  if (!p.title) return;
  try {
    const t = await post('/api/tasks', { title: p.title, due: p.due, tags: p.tags, starred: p.starred, repeat: p.repeat, list_id: S?.listId });
    toast(`Added “${t.title}”${t.due ? ` · ${relDay(t.due)}` : ''}`);
    if (S) load();
  } catch (e) { fail(e); }
}

function flash(id) {
  requestAnimationFrame(() => {
    const el = $(`.task[data-id="${id}"]`, S.root);
    el?.animate([{ background: 'var(--accent-soft)' }, { background: 'transparent' }], { duration: 1100, easing: 'ease-out' });
  });
}

// ── Rendering ──

function visibleOpen() {
  let ts = S.open;
  if (S.tag) ts = ts.filter((t) => t.tags.includes(S.tag));
  if (S.day && S.sort === 'date') ts = ts.filter((t) => t.due === S.day);
  return ts;
}

function renderAll() {
  if (!S) return;
  renderRibbon();
  renderTagbar();
  renderList();
  renderDone();
}

function renderRibbon() {
  if (S.sort !== 'date') { S.el.ribbon.replaceChildren(); S.el.ribbon.hidden = true; return; }
  S.el.ribbon.hidden = false;
  const t0 = today();
  const counts = {};
  let overdue = 0;
  for (const t of S.open) {
    if (!t.due) continue;
    if (t.due < iso(t0)) overdue++;
    counts[t.due] = (counts[t.due] || 0) + 1;
  }
  const cells = [];
  for (let i = 0; i < 15; i++) {
    const d = addDays(t0, i);
    const k = iso(d);
    const n = counts[k] || 0;
    const isToday = i === 0;
    cells.push(h('button.day', {
      'data-day': k,
      class: [isToday && 'today', (d.getDay() === 0 || d.getDay() === 6) && 'weekend', S.day === k && 'sel'].filter(Boolean).join(' '),
      title: `${fmtShort(d)} — ${n} task${n === 1 ? '' : 's'}${isToday && overdue ? `, ${overdue} overdue` : ''}. Drop a task here to move it.`,
      onclick: () => { S.day = S.day === k ? null : k; renderAll(); },
    },
    h('span.wd', isToday ? 'Today' : fmtWd(d).slice(0, 2)),
    h('span.dn', d.getDate()),
    h('span.load', Array.from({ length: Math.min(n, 4) }, () => h('i')))));
  }
  fill(S.el.ribbon, ...cells);
}

function renderTagbar() {
  const counts = {};
  S.open.forEach((t) => t.tags.forEach((g) => { counts[g] = (counts[g] || 0) + 1; }));
  const tags = Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  if (!tags.length) { S.el.tagbar.replaceChildren(); return; }
  fill(S.el.tagbar, 
    h('button', { class: !S.tag ? 'on' : '', onclick: () => { S.tag = null; renderAll(); } }, 'All'),
    ...tags.map(([g, n]) => h('button', { class: S.tag === g ? 'on' : '', onclick: () => { S.tag = S.tag === g ? null : g; renderAll(); } }, `#${g}`, h('small', n))));
}

function groupsByDate(ts) {
  const t0 = iso(today());
  const tomorrow = iso(addDays(today(), 1));
  const groups = new Map();
  const add = (key, label, cls, t) => {
    if (!groups.has(key)) groups.set(key, { key, label, cls, items: [] });
    groups.get(key).items.push(t);
  };
  const sorted = [...ts].sort((a, b) => (a.due ?? '9999') .localeCompare(b.due ?? '9999') || b.starred - a.starred || a.position - b.position);
  for (const t of sorted) {
    if (!t.due) add('none', 'No date', '', t);
    else if (t.due < t0) add('past', 'Past', 'past', t);
    else if (t.due === t0) add(t0, 'Today', 'today', t);
    else if (t.due === tomorrow) add(t.due, 'Tomorrow', '', t);
    else {
      const d = parseISO(t.due);
      add(t.due, d.getFullYear() === today().getFullYear() ? fmtShort(d) : fmtLong(d), '', t);
    }
  }
  return [...groups.values()];
}

function renderList() {
  const ts = visibleOpen();
  S.el.list.replaceChildren();
  if (!ts.length) {
    S.el.list.append(emptyState());
    return;
  }
  if (S.sort === 'date') {
    for (const g of groupsByDate(ts)) {
      S.el.list.append(h('section.group', { 'aria-label': g.label },
        h('div.group-h', { class: g.cls }, g.label, h('span.n', g.items.length),
          g.key === 'past' && g.items.length > 1 ? h('button', { onclick: () => moveAll(g.items, iso(today())), title: 'Move every overdue task to today' }, 'Move all to today') : null),
        g.items.map((t) => row(t, g.key))));
    }
  } else {
    const sorted = [...ts].sort((a, b) => a.position - b.position);
    S.el.list.append(h('section.group', sorted.map((t) => row(t, 'custom'))));
  }
}

function emptyState() {
  if (S.tag || S.day) {
    return h('div.empty', h('div.art', '∅'), h('h3', 'Nothing here'), h('p', S.day ? `No tasks on ${relDay(S.day)}. Type above to add one to that day.` : `No open tasks tagged #${S.tag}.`));
  }
  const doneToday = S.done.filter((t) => t.completed_at && dayDiff(new Date(t.completed_at), today()) === 0).length;
  return h('div.empty', h('div.art', '✓'),
    h('h3', doneToday ? `All clear — ${doneToday} done today` : 'All clear'),
    h('p', 'Add a task above. Dates, #tags, “every week” and “!” for a star are understood as you type.'));
}

function renderDone() {
  const n = S.done.length;
  if (!n) { S.el.done.replaceChildren(); return; }
  const toggle = h('button.completed-toggle', { class: S.showDone ? 'open' : '', 'aria-expanded': S.showDone, onclick: () => { S.showDone = !S.showDone; renderDone(); } },
    icon('right'), `Completed (${n}${n >= 60 ? '+' : ''})`);
  fill(S.el.done, toggle, S.showDone ? h('section.group', S.done.map((t) => row(t, 'done'))) : null);
}

function dueChip(t, groupKey) {
  if (!t.due) return null;
  const n = dayDiff(parseISO(t.due), today());
  const show = groupKey === 'custom' || groupKey === 'done' || groupKey === 'past' || n === 0 || t.repeat || groupKey === 'none';
  if (!show) return t.repeat ? h('span.due', icon('repeat')) : null;
  const cls = n < 0 && !t.completed_at ? 'past' : n === 0 ? 'today' : '';
  if (t.completed_at) {
    return h('span.due', { class: cls, title: REPEAT_LABEL[t.repeat] || '' }, icon('clock'), relDay(t.due), t.repeat ? icon('repeat') : null);
  }
  // Clickable: opens the native date picker and saves the new due date.
  const input = h('input.due-input', { type: 'date', value: t.due, tabIndex: -1, 'aria-hidden': 'true' });
  input.addEventListener('change', async () => {
    if (!input.value || input.value === t.due) return;
    try {
      const nt = await patch(`/api/tasks/${t.id}`, { due: input.value });
      Object.assign(t, nt);
      renderAll();
    } catch (e) { fail(e); }
  });
  return h('button.due.pick', {
    class: cls, title: [REPEAT_LABEL[t.repeat], 'Change due date'].filter(Boolean).join(' · '), 'aria-label': `Due ${relDay(t.due)}. Change date`,
    onclick: (e) => { e.stopPropagation(); try { input.showPicker(); } catch { input.focus(); input.click(); } },
  }, icon('clock'), relDay(t.due), t.repeat ? icon('repeat') : null, input);
}

function row(t, groupKey) {
  if (t.id === S.openId) return editorRow(t);
  const done = !!t.completed_at;
  const el = h('div.task', {
    role: 'listitem', tabIndex: -1, 'data-id': t.id,
    class: [done && 'done', S.focusId === t.id && 'focus'].filter(Boolean).join(' '),
    onclick: (e) => {
      if (S.justDragged || e.target.closest('.check, .star, .grip, a')) return;
      if (!done) openEditor(t.id);
    },
  },
  h('button.check', { 'aria-label': done ? `Mark “${t.title}” not done` : `Complete “${t.title}”`, onclick: () => (done ? reopen(t) : complete(t, el)) }, icon('check')),
  h('div.body',
    h('div.title', t.title),
    h('div.sub',
      dueChip(t, groupKey),
      t.countdown_title ? h('a.milepill', { href: '/countdown', title: `Part of the plan for ${t.countdown_title}` },
        icon('hourglass'), t.countdown_title, h('span', milestoneLabel(t.days_before))) : null,
      t.tags.map((g) => h('span.tagpill', `#${g}`)),
      // Provenance stays quiet in the list (a ✦ with a tooltip); the editor names the author.
      t.created_by !== 'you' ? h('span.spark', { title: `Added by ${t.created_by}`, 'aria-label': `Added by ${t.created_by}` }, '✦') : null,
      t.notes ? h('span.notes', t.notes.split('\n')[0]) : null)),
  h('div.side',
    !done ? h('button.star', { class: t.starred ? 'on' : '', 'aria-label': t.starred ? 'Unstar' : 'Star', 'aria-pressed': t.starred, onclick: () => toggleStar(t) }, icon('star')) : null,
    !done ? h('span.grip', { title: 'Drag to reorder or onto a day', 'aria-hidden': 'true' }, icon('grip')) : null));
  if (!done) el.addEventListener('pointerdown', (e) => startDrag(e, t, el));
  return el;
}

// ── Inline editor ──

function openEditor(id) {
  if (S.openId === id) return;
  if (S.openId) closeEditor(false);
  S.openId = id;
  S.focusId = id;
  renderList();
  requestAnimationFrame(() => $('.task.open .t', S.root)?.focus());
}

function closeEditor(rerender = true) {
  const save = S.saveNow;
  S.saveNow = null;
  save?.();
  S.openId = null;
  if (rerender) {
    if (S.deferred) { S.deferred = false; load(); } else renderAll();
  }
}

function editorRow(t) {
  const draft = { ...t, tags: [...t.tags] };
  const pending = {};
  const push = debounce(async () => {
    const body = { ...pending };
    for (const k of Object.keys(pending)) delete pending[k];
    if (!Object.keys(body).length) return;
    try {
      const nt = await patch(`/api/tasks/${t.id}`, body);
      const i = S.open.findIndex((x) => x.id === t.id);
      if (i >= 0) S.open[i] = nt;
      if (nt.list_id !== S.listId) { S.open.splice(i, 1); toast(`Moved to ${S.lists.find((l) => l.id === nt.list_id)?.name}`); }
    } catch (e) { fail(e); }
  }, 350);
  const set = (k, v) => { draft[k] = v; pending[k] = v; push(); };
  S.saveNow = () => push.flush();

  const title = h('input.t', { value: t.title, 'aria-label': 'Title', maxLength: 500 });
  title.addEventListener('input', () => title.value.trim() && set('title', title.value));
  title.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); closeEditor(); } });
  const notes = autosize(h('textarea.n', { rows: 1, placeholder: 'Details', value: t.notes, 'aria-label': 'Details' }));
  notes.addEventListener('input', () => set('notes', notes.value));

  const date = h('input.input', { type: 'date', value: t.due || '', 'aria-label': 'Due date' });
  date.addEventListener('change', () => { set('due', date.value || null); dueLabel(); });
  const quick = (label, d) => h('button.btn.sm.ghost', { onclick: () => { date.value = d ? iso(d) : ''; set('due', d ? iso(d) : null); dueLabel(); } }, label);
  const dueText = h('span.muted', { style: { fontSize: '12px' } });
  const dueLabel = () => { dueText.textContent = date.value ? relDay(date.value) : ''; };
  dueLabel();

  const repeat = h('select.input', { 'aria-label': 'Repeat' },
    h('option', { value: '' }, 'Doesn’t repeat'),
    Object.entries(REPEAT_LABEL).map(([v, l]) => h('option', { value: v, selected: t.repeat === v }, l)));
  repeat.addEventListener('change', () => {
    set('repeat', repeat.value || null);
    if (repeat.value && !date.value) { date.value = iso(today()); dueLabel(); }
  });

  const listSel = S.lists.length > 1 ? h('select.input', { 'aria-label': 'List', onchange: (e) => set('list_id', e.target.value) },
    S.lists.map((l) => h('option', { value: l.id, selected: l.id === t.list_id }, l.name))) : null;

  const tagWrap = h('div.tagedit');
  const tagIn = h('input', { placeholder: draft.tags.length ? '' : 'Add tags', 'aria-label': 'Add tag' });
  const drawTags = () => fill(tagWrap, ...draft.tags.map((g) => h('span.pchip.tag', `#${g}`,
    h('button', { 'aria-label': `Remove ${g}`, onclick: () => { set('tags', draft.tags.filter((x) => x !== g)); drawTags(); } }, icon('x')))), tagIn);
  tagIn.addEventListener('keydown', (e) => {
    if (['Enter', ',', ' ', 'Tab'].includes(e.key) && tagIn.value.trim()) {
      e.preventDefault();
      const g = tagIn.value.trim().replace(/^#/, '').toLowerCase().replace(/\s+/g, '-');
      if (!draft.tags.includes(g)) set('tags', [...draft.tags, g]);
      tagIn.value = '';
      drawTags(); tagIn.focus();
    } else if (e.key === 'Backspace' && !tagIn.value && draft.tags.length) {
      set('tags', draft.tags.slice(0, -1)); drawTags(); tagIn.focus();
    }
  });
  drawTags();

  const star = h('button.btn.sm', { class: draft.starred ? 'on' : '', 'aria-pressed': draft.starred, style: draft.starred ? { color: 'var(--star)' } : null,
    onclick: (e) => { set('starred', !draft.starred); e.currentTarget.style.color = draft.starred ? 'var(--star)' : ''; e.currentTarget.setAttribute('aria-pressed', draft.starred); } },
    icon('star'), 'Star');

  const el = h('div.task.open', { 'data-id': t.id, role: 'listitem' },
    h('button.check', { 'aria-label': `Complete “${t.title}”`, onclick: () => { closeEditor(false); complete(draft, el); } }, icon('check')),
    h('div.editor',
      title, notes,
      h('div.ctrls', icon('cal'), date, h('div.quick', quick('Today', today()), quick('Tomorrow', addDays(today(), 1)), quick('Next week', addDays(today(), 7)), quick('Clear', null)), dueText),
      h('div.ctrls', icon('repeat'), repeat, listSel, star),
      h('div.ctrls', icon('tag'), tagWrap),
      h('div.foot',
        t.created_by !== 'you' ? h('span.by-ai', '✦ Added by ', t.created_by) : null,
        h('span', `Created ${ago(t.created_at)}`),
        h('span.grow'),
        h('button.btn.sm.ghost', { onclick: () => closeEditor() }, 'Done'),
        h('button.btn.sm.ghost.danger', { onclick: () => { closeEditor(false); remove(t); } }, icon('trash'), 'Delete'))));
  el.addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.stopPropagation(); closeEditor(); focusRow(t.id); } });
  return el;
}

// ── Mutations ──

async function complete(t, el) {
  el?.classList.add('completing');
  try {
    const r = await post(`/api/tasks/${t.id}/complete`);
    await new Promise((res) => setTimeout(res, 260));
    if (r.rolled_to) {
      toast(`Done · next ${relDay(r.rolled_to)}`);
      const i = S.open.findIndex((x) => x.id === t.id);
      if (i >= 0) S.open[i] = r.task;
    } else {
      S.open = S.open.filter((x) => x.id !== t.id);
      S.done.unshift(r.task);
      toast(`Completed “${trim(t.title)}”`, { action: 'Undo', onAction: () => reopen(r.task) });
    }
    advanceFocus(t.id);
    renderAll();
    ctx.refreshNav();
  } catch (e) { el?.classList.remove('completing'); fail(e); }
}

async function reopen(t) {
  try {
    const nt = await post(`/api/tasks/${t.id}/uncomplete`);
    S.done = S.done.filter((x) => x.id !== t.id);
    S.open.push(nt);
    renderAll();
    flash(nt.id);
  } catch (e) { fail(e); }
}

async function remove(t) {
  try {
    await del(`/api/tasks/${t.id}`);
    S.open = S.open.filter((x) => x.id !== t.id);
    S.done = S.done.filter((x) => x.id !== t.id);
    advanceFocus(t.id);
    renderAll();
    toast(`Deleted “${trim(t.title)}”`, { action: 'Undo', onAction: async () => {
      const nt = await post('/api/tasks', { list_id: t.list_id, title: t.title, notes: t.notes, tags: t.tags, due: t.due, repeat: t.repeat, starred: t.starred });
      if (t.position !== undefined) await post(`/api/tasks/${nt.id}/move`, { after: neighborBefore(t) }).catch(() => {});
      load();
    } });
  } catch (e) { fail(e); }
}

function neighborBefore(t) {
  const before = S.open.filter((x) => x.position < t.position).sort((a, b) => b.position - a.position)[0];
  return before?.id ?? null;
}

async function toggleStar(t) {
  try {
    const nt = await patch(`/api/tasks/${t.id}`, { starred: !t.starred });
    Object.assign(t, nt);
    renderList();
  } catch (e) { fail(e); }
}

async function moveAll(items, due) {
  try {
    await Promise.all(items.map((t) => patch(`/api/tasks/${t.id}`, { due })));
    toast(`Moved ${items.length} tasks to today`);
    load();
  } catch (e) { fail(e); }
}

const trim = (s) => (s.length > 40 ? s.slice(0, 38) + '…' : s);

// ── Drag: reorder (My order) or reschedule onto the ribbon ──

function startDrag(e, t, el) {
  if (e.button !== 0 || S.openId) return;
  if (e.target.closest('.check, .star, button, input, textarea, select')) return;
  if (e.pointerType !== 'mouse' && !e.target.closest('.grip')) return;
  const x0 = e.clientX, y0 = e.clientY;
  let ghost = null, line = null, dropDay = null, dropAfter;

  const move = (ev) => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      S.dragging = true;
      ghost = h('div.drag-ghost', t.title);
      document.body.append(ghost);
      el.classList.add('dragging');
      document.body.style.userSelect = 'none';
    }
    ev.preventDefault();
    ghost.style.left = ev.clientX + 14 + 'px';
    ghost.style.top = ev.clientY - 18 + 'px';
    const under = document.elementFromPoint(ev.clientX, ev.clientY);
    const day = under?.closest('.ribbon .day');
    $$('.ribbon .day.drop').forEach((d) => d !== day && d.classList.remove('drop'));
    dropDay = day?.dataset.day ?? null;
    day?.classList.add('drop');
    line?.remove(); line = null; dropAfter = undefined;
    if (!day && S.sort === 'custom') {
      const rows = $$('.task:not(.done):not(.dragging)', S.el.list);
      let after = null, anchor = rows[0];
      for (const r of rows) {
        const b = r.getBoundingClientRect();
        if (ev.clientY > b.top + b.height / 2) { after = r.dataset.id; anchor = r.nextSibling; } else { anchor = r; break; }
      }
      if (rows.length) {
        line = h('div.drop-line');
        anchor ? anchor.parentNode.insertBefore(line, anchor) : rows.at(-1).after(line);
        dropAfter = after;
      }
    }
  };
  const up = async () => {
    removeEventListener('pointermove', move);
    removeEventListener('pointerup', up);
    removeEventListener('pointercancel', up);
    if (!ghost) return;
    ghost.remove(); line?.remove(); el.classList.remove('dragging');
    document.body.style.userSelect = '';
    $$('.ribbon .day.drop').forEach((d) => d.classList.remove('drop'));
    S.justDragged = true; setTimeout(() => { if (S) S.justDragged = false; }, 50);
    S.dragging = false;
    try {
      if (dropDay) {
        await patch(`/api/tasks/${t.id}`, { due: dropDay });
        toast(`Moved to ${relDay(dropDay)}`);
      } else if (dropAfter !== undefined && dropAfter !== t.id) {
        await post(`/api/tasks/${t.id}/move`, { after: dropAfter });
      }
    } catch (err) { fail(err); }
    S.deferred = false;
    load();
  };
  addEventListener('pointermove', move, { passive: false });
  addEventListener('pointerup', up);
  addEventListener('pointercancel', up);
}

// ── Keyboard ──

function visibleRows() { return $$('.task:not(.done)', S.el.list); }
function focusRow(id) {
  S.focusId = id;
  $$('.task', S.root).forEach((r) => r.classList.toggle('focus', r.dataset.id === id));
  const el = $(`.task[data-id="${id}"]`, S.root);
  el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}
function advanceFocus(id) {
  if (S.focusId !== id) return;
  const rows = visibleRows().map((r) => r.dataset.id);
  const i = rows.indexOf(id);
  S.focusId = rows[i + 1] ?? rows[i - 1] ?? null;
}

async function onKey(e) {
  if (!S || S.openId) return;
  const rows = visibleRows().map((r) => r.dataset.id);
  const i = rows.indexOf(S.focusId);
  const t = S.open.find((x) => x.id === S.focusId);
  if (e.key === 'n' || e.key === 'c') { e.preventDefault(); S.el.composer.focusInput(); }
  else if (e.key === 'j' || e.key === 'ArrowDown' && !e.altKey) { e.preventDefault(); if (rows.length) focusRow(rows[Math.min(i + 1, rows.length - 1)]); }
  else if (e.key === 'k' || e.key === 'ArrowUp' && !e.altKey) { e.preventDefault(); if (rows.length) focusRow(rows[Math.max(i - 1, 0)]); }
  else if (!t) return;
  else if (e.key === 'x' || e.key === ' ') { e.preventDefault(); complete(t, $(`.task[data-id="${t.id}"]`, S.root)); }
  else if (e.key === 'Enter' || e.key === 'e') { e.preventDefault(); openEditor(t.id); }
  else if (e.key === 's') { e.preventDefault(); toggleStar(t); }
  else if (e.key === 'Backspace' || e.key === 'Delete') { e.preventDefault(); remove(t); }
  else if (e.key === 'Escape') { S.focusId = null; S.tag = null; S.day = null; renderAll(); }
  else if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown') && S.sort === 'custom') {
    e.preventDefault();
    const order = [...S.open].sort((a, b) => a.position - b.position).map((x) => x.id);
    const j = order.indexOf(t.id);
    const without = order.filter((x) => x !== t.id);
    const target = e.key === 'ArrowUp' ? Math.max(j - 1, 0) : Math.min(j + 1, without.length);
    const after = target === 0 ? null : without[target - 1];
    try { const nt = await post(`/api/tasks/${t.id}/move`, { after }); Object.assign(t, nt); renderList(); focusRow(t.id); } catch (err) { fail(err); }
  }
}

function reveal(t) {
  if (!S) return;
  if (t.completed_at) { S.showDone = true; }
  if (t.list_id !== S.listId) { switchList(t.list_id); setTimeout(() => reveal(t), 300); return; }
  S.tag = null; S.day = null;
  if (!t.completed_at) openEditor(t.id); else { renderAll(); focusRow(t.id); flash(t.id); }
}
