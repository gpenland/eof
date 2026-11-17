// App shell: auth gate, router, navigation with live meta, change feed, palette, shortcuts.

import { h, icon, get, post, put, toast, fail, debounce, isTyping, store, $, $$, closeMenus, fill, relDay } from './util.js';
import * as Tasks from './tasks.js';
import * as Countdown from './countdown.js';
import * as News from './news.js';
import * as Settings from './settings.js';
import * as Lock from './login.js';

export const ctx = {
  boot: null, // { today, timezone, lists, mcp_url, origin }
  page: null,
  navigate,
  refreshNav: () => {},
};

const PAGES = [
  { path: '/tasks', label: 'Tasks', icon: 'tasks', mod: Tasks, key: '1' },
  { path: '/countdown', label: 'Countdown', icon: 'hourglass', mod: Countdown, key: '2' },
  { path: '/news', label: 'News', icon: 'paper', mod: News, key: '3' },
];

applyTheme(store.get('theme', 'system'));
export function applyTheme(t) {
  if (t === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = t;
}

// ── Routing ──

export function navigate(path, { replace = false } = {}) {
  if (path === location.pathname + location.search) return route();
  history[replace ? 'replaceState' : 'pushState']({}, '', path);
  route();
}
addEventListener('popstate', () => route());
document.addEventListener('click', (e) => {
  const a = e.target.closest('a[href^="/"]');
  if (!a || e.metaKey || e.ctrlKey || e.shiftKey || a.target === '_blank') return;
  e.preventDefault();
  navigate(a.getAttribute('href'));
});
addEventListener('eof:signed-out', () => {
  if (!location.pathname.startsWith('/login')) navigate('/login', { replace: true });
});

let shell = null;
let current = null;

async function route() {
  closeMenus();
  const app = $('#app');
  const path = location.pathname;
  const status = await get('/auth/status').catch(() => ({ signed_in: false, needs_setup: false }));

  if (path === '/login' || !status.signed_in) {
    teardown();
    if (path !== '/login') {
      const next = location.pathname + location.search;
      history.replaceState({}, '', '/login' + (next !== '/' ? `?next=${encodeURIComponent(next)}` : ''));
    }
    if (status.signed_in) return navigate(safeNext(), { replace: true });
    fill(app, Lock.lockScreen(status, () => { ctx.boot = null; navigate(safeNext(), { replace: true }); }));
    return ready();
  }
  if (path === '/connect') {
    teardown();
    fill(app, Lock.consentScreen());
    return ready();
  }
  if (!ctx.boot) await boot();
  if (!shell) buildShell(app);

  const page = path.startsWith('/news') ? PAGES[2]
    : path === '/settings' ? { path: '/settings', label: 'Settings', mod: Settings }
    : PAGES.find((p) => p.path === path) ?? PAGES[0];
  if (path === '/') history.replaceState({}, '', '/tasks');

  $$('.nav a.item').forEach((a) => a.classList.toggle('on', a.getAttribute('href') === page.path));
  document.title = page.label ? `${page.label} · EOF` : 'EOF';
  const main = $('main');
  if (current?.mod === page.mod && current.inst?.reroute) {
    current.inst.reroute(path);
  } else {
    current?.inst?.unmount?.();
    main.replaceChildren();
    window.scrollTo(0, 0);
    current = { mod: page.mod, inst: page.mod.mount(main, path) };
  }
  ctx.page = current.inst;
  ready();
}

function safeNext() {
  const n = new URLSearchParams(location.search).get('next') || '/tasks';
  return n.startsWith('/') && !n.startsWith('//') && !n.startsWith('/login') ? n : '/tasks';
}
function ready() { $('#app').classList.remove('booting'); }
function teardown() {
  current?.inst?.unmount?.();
  current = null;
  shell = null;
  feed?.close();
  feed = null;
}

async function boot() {
  ctx.boot = await get('/api/bootstrap');
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
  if (tz && tz !== ctx.boot.timezone) {
    await put('/api/timezone', { timezone: tz }).catch(() => {});
    ctx.boot.timezone = tz;
  }
  connectFeed();
}

// ── Shell & nav ──

function buildShell(app) {
  const metas = {};
  const nav = h('nav.nav', { 'aria-label': 'Main' },
    h('a.wordmark', { href: '/tasks', 'aria-label': 'EOF home' }, 'EOF', h('span.caret')),
    PAGES.map((p) => h('a.item', { href: p.path, title: `${p.label} (${p.key})` },
      icon(p.icon), h('span', p.label), metas[p.path] = h('span.meta'))),
    h('div.spacer'),
    h('button.hint', { onclick: openPalette }, icon('search'), h('span', { style: { flex: 1, textAlign: 'left' } }, 'Search & commands'), h('kbd', '⌘K')),
    h('a.item', { href: '/settings', title: 'Settings' }, icon('gear'), h('span', 'Settings'), h('span.meta')),
  );
  shell = h('div.shell', nav, h('main', { id: 'main' }));
  fill(app, shell);

  ctx.refreshNav = debounce(async () => {
    try {
      const o = await get('/api/overview');
      const due = o.tasks.overdue.length + o.tasks.today.length;
      setMeta(metas['/tasks'], due ? String(due) : '', o.tasks.overdue.length > 0);
      const next = o.countdowns.find((c) => c.days_until >= 0);
      setMeta(metas['/countdown'], next ? (next.days_until === 0 ? 'now' : `${next.days_until}d`) : '');
      const li = o.latest_issue;
      metas['/news'].className = 'meta' + (li && !li.read ? ' dot' : '');
      metas['/news'].textContent = '';
      metas['/news'].title = li && !li.read ? 'Unread edition' : '';
    } catch { /* signed out mid-flight */ }
  }, 120);
  ctx.refreshNav();
}
function setMeta(el, text, hot = false) {
  el.textContent = text;
  el.className = 'meta' + (hot ? ' hot' : '');
}

// ── Live change feed ──

let feed = null;
function connectFeed() {
  feed?.close();
  feed = new EventSource('/api/events');
  feed.addEventListener('change', (e) => {
    const ev = JSON.parse(e.data);
    if (ev.actor !== 'you') toast(`${ev.actor} ${ev.message}`, { ai: true, ms: 5200 });
    ctx.page?.onChange?.(ev);
    ctx.refreshNav();
  });
  // EventSource reconnects by itself; after a *re*connect, re-sync everything we may have missed.
  let connectedOnce = false;
  feed.addEventListener('open', () => {
    if (connectedOnce) { ctx.page?.onChange?.({ area: '*' }); ctx.refreshNav(); }
    connectedOnce = true;
  });
}
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && shell) { ctx.page?.onChange?.({ area: '*' }); ctx.refreshNav(); }
});

// ── Keyboard ──

document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); if (shell) openPalette(); return; }
  if (!shell || isTyping(e) || e.metaKey || e.ctrlKey || e.altKey && !e.key.startsWith('Arrow')) return;
  if ($('.palette, .sheet-backdrop, .menu')) return;
  const p = PAGES.find((x) => x.key === e.key);
  if (p) { e.preventDefault(); return navigate(p.path); }
  if (e.key === '/') { e.preventDefault(); return openPalette(); }
  if (e.key === '?') { e.preventDefault(); return showHelp(); }
  ctx.page?.key?.(e);
});

function showHelp() {
  import('./util.js').then(({ sheet }) => sheet((box, close) => {
    const row = (k, d) => [h('span', k.split(' ').map((x) => h('kbd', x)), ' '), h('span', d)];
    box.append(h('h2', 'Keyboard'), h('div.help-grid',
      row('⌘ K', 'Search & commands'), row('1 2 3', 'Tasks · Countdown · News'),
      row('n', 'New task / new date'), row('j k', 'Move between tasks'), row('x', 'Complete task'),
      row('Enter', 'Open task'), row('s', 'Star task'), row('⌥ ↑ ⌥ ↓', 'Reorder (My order)'),
      row('← →', 'Previous / next edition'), row('?', 'This help')),
      h('div.actions', h('button.btn', { onclick: close }, 'Done')));
  }));
}

// ── Command palette ──

function openPalette() {
  if ($('.palette')) return;
  const input = h('input', { placeholder: 'Jump to, search tasks, or type a new task…', 'aria-label': 'Command' });
  const res = h('div.res', { role: 'listbox' });
  const back = h('div.palette', { onmousedown: (e) => { if (e.target === back) close(); } }, h('div.box', input, res));
  let items = [];
  let sel = 0;
  const statics = [
    { label: 'Tasks', hint: '1', icon: 'tasks', run: () => navigate('/tasks') },
    { label: 'Countdown', hint: '2', icon: 'hourglass', run: () => navigate('/countdown') },
    { label: 'Today’s edition', hint: '3', icon: 'paper', run: () => navigate('/news') },
    { label: 'New countdown…', icon: 'plus', run: () => { navigate('/countdown'); setTimeout(() => ctx.page?.create?.(), 50); } },
    { label: 'Settings', icon: 'gear', run: () => navigate('/settings') },
    { label: 'Connect an AI…', icon: 'link', run: () => navigate('/settings#ai') },
  ];
  function render(found = []) {
    const q = input.value.trim().toLowerCase();
    const st = statics.filter((s) => !q || s.label.toLowerCase().includes(q));
    items = [];
    res.replaceChildren();
    if (q) {
      items.push({ label: `Add task “${input.value.trim()}”`, icon: 'plus', run: () => Tasks.quickAdd(input.value.trim()) });
    }
    items.push(...st);
    if (found.length) items.push({ section: 'Tasks' }, ...found);
    let i = 0;
    for (const it of items) {
      if (it.section) { res.append(h('div.sec', it.section)); continue; }
      const idx = i++;
      it.idx = idx;
      res.append(h('button.it', { class: idx === sel ? 'on' : '', onmousemove: () => hl(idx), onclick: () => runIt(it) },
        icon(it.icon || 'right'), h('span.grow', it.label), it.hint ? h('small', it.hint) : null));
    }
  }
  const hl = (i) => { sel = i; $$('.it', res).forEach((b, j) => b.classList.toggle('on', j === i)); };
  const runIt = (it) => { close(); it.run(); };
  const search = debounce(async () => {
    const q = input.value.trim();
    if (q.length < 2) return render();
    const ts = await get(`/api/tasks?status=all&limit=8&search=${encodeURIComponent(q)}`).catch(() => []);
    render(ts.map((t) => ({ label: t.title, hint: t.completed_at ? 'done' : (t.due ? relDay(t.due) : ''), icon: t.completed_at ? 'check' : 'tasks', run: () => { navigate('/tasks'); setTimeout(() => ctx.page?.reveal?.(t), 60); } })));
  }, 140);
  input.addEventListener('input', () => { sel = 0; render(); search(); });
  input.addEventListener('keydown', (e) => {
    const n = items.filter((x) => !x.section).length;
    if (e.key === 'ArrowDown') { e.preventDefault(); hl((sel + 1) % n); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); hl((sel - 1 + n) % n); }
    else if (e.key === 'Enter') { e.preventDefault(); const it = items.find((x) => x.idx === sel); if (it) runIt(it); }
    else if (e.key === 'Escape') close();
  });
  function close() { back.remove(); }
  document.body.append(back);
  render();
  input.focus();
}

route();
