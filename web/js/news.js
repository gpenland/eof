// News — one calm edition per day. A broadsheet masthead, a book-like column, a section map that
// follows you, a hairline progress bar, and an end mark that files the edition as read.

import { h, icon, get, post, put, toast, fail, menu, store, $, $$, parseISO, fmtFull, fmtMD, fmtTime, fmtWd, fill, iso, today, relDay, debounce } from './util.js';
import { ctx, navigate } from './app.js';

let S = null;

export function mount(root, path) {
  S = {
    root, issues: [], issue: null,
    prefs: store.get('reading', { size: 19, sans: false }),
    progress: h('div.progress', { 'aria-hidden': 'true' }),
    shelf: h('nav.shelf', { 'aria-label': 'Editions' }),
    mshelf: h('nav.mobile-shelf', { 'aria-label': 'Editions' }),
    article: h('article.issue'),
    yourday: h('section.yourday', { 'aria-label': 'Your day' }),
    toc: h('nav.toc', { 'aria-label': 'Sections' }),
    observers: [],
  };
  document.body.append(S.progress);
  root.append(h('div.edition', S.shelf, h('div', S.mshelf, S.article), S.toc));
  addEventListener('scroll', onScroll, { passive: true });
  init(path);
  return {
    unmount() { removeEventListener('scroll', onScroll); S.progress.remove(); S.observers.forEach((o) => o.disconnect()); S = null; },
    reroute: (p) => openIssue(whichFrom(p)),
    onChange(ev) {
      if (['tasks', 'countdowns', '*'].includes(ev.area)) refreshDay();
      if (ev.area !== 'news' && ev.area !== '*') return;
      loadShelf().then(() => {
        if (!S) return;
        // A fresh edition arrived while looking at the latest one: show it.
        if (location.pathname === '/news' && S.issues[0] && S.issue?.edition !== S.issues[0].edition) openIssue('latest');
      });
    },
    key(e) {
      if (e.key === 'ArrowLeft' && S.issue?.prev) navigate(`/news/${S.issue.prev}`);
      if (e.key === 'ArrowRight' && S.issue?.next) navigate(`/news/${S.issue.next}`);
    },
  };
}

const whichFrom = (p) => (p.match(/^\/news\/(\d{4}-\d{2}-\d{2})$/)?.[1] ?? 'latest');

async function init(path) {
  await loadShelf();
  if (!S) return;
  if (!S.issues.length) return renderEmpty();
  openIssue(whichFrom(path));
}

async function loadShelf() {
  try { S.issues = await get('/api/issues?limit=120'); renderShelf(); } catch (e) { fail(e); }
}

function renderShelf() {
  if (!S) return;
  const cur = S.issue?.edition;
  let month = null;
  const items = [];
  for (const i of S.issues) {
    const d = parseISO(i.edition);
    const m = d.toLocaleDateString(undefined, { month: 'long', year: 'numeric' });
    if (m !== month) { items.push(h('div.month', m)); month = m; }
    items.push(h('a', { href: `/news/${i.edition}`, class: i.edition === cur ? 'on' : '', title: i.title },
      h('span.d', `${fmtWd(d)} ${d.getDate()}`), i.read ? null : h('span.u', { title: 'Unread' })));
  }
  fill(S.shelf, h('h4', 'Editions'), ...items);
  fill(S.mshelf, ...S.issues.slice(0, 30).map((i) => {
    const d = parseISO(i.edition);
    return h('a', { href: `/news/${i.edition}`, class: i.edition === cur ? 'on' : '' }, fmtWd(d), h('b', d.getDate()), i.read ? null : h('span.u'));
  }));
  $('.mobile-shelf a.on', S.root)?.scrollIntoView({ inline: 'center', block: 'nearest' });
}

async function openIssue(which) {
  try {
    const issue = await get(`/api/issues/${which}`);
    if (!S) return;
    S.issue = issue;
    renderIssue();
    renderShelf();
    window.scrollTo(0, 0);
  } catch (e) {
    if (e.status === 404) { fill(S.article, h('div.empty', h('div.art', '—'), h('h3', 'No edition that day'), h('p', h('a', { href: '/news' }, 'Go to the latest edition')))); S.toc.replaceChildren(); }
    else fail(e);
  }
}

function applyPrefs() {
  S.article.style.setProperty('--fs', `${S.prefs.size}px`);
  S.article.classList.toggle('sans', S.prefs.sans);
  store.set('reading', S.prefs);
}

function renderIssue() {
  const i = S.issue;
  const d = parseISO(i.edition);
  const published = new Date(i.published_at);
  const prose = h('div.prose', { html: i.html }); // sanitized server-side by ammonia
  // Section anchors + external links open in a new tab.
  $$('h2', prose).forEach((el, n) => { el.id = `s${n + 1}`; });
  // Drop cap only on a real opening paragraph; a short line like "Today" lets the float spill into the list below.
  const lead = $('p', prose);
  if (lead && lead.parentElement === prose && lead.textContent.trim().length > 140) lead.classList.add('dropcap');
  $$('a[href^="http"]', prose).forEach((a) => { a.target = '_blank'; a.rel = 'noopener noreferrer'; });

  const end = h('div.endmark', h('div.eof', 'EOF'), h('p', i.read_at ? 'You read this edition.' : 'That’s the edition. You’re caught up.'));
  const aa = h('button.btn.sm.ghost', { 'aria-label': 'Reading settings', title: 'Reading settings', onclick: (e) => menu(e.currentTarget, [
    { section: 'Text' },
    { label: 'Larger', icon: 'plus', onClick: () => { S.prefs.size = Math.min(24, S.prefs.size + 1); applyPrefs(); } },
    { label: 'Smaller', icon: 'type', onClick: () => { S.prefs.size = Math.max(15, S.prefs.size - 1); applyPrefs(); } },
    { label: 'Serif', check: !S.prefs.sans, onClick: () => { S.prefs.sans = false; applyPrefs(); } },
    { label: 'Sans-serif', check: S.prefs.sans, onClick: () => { S.prefs.sans = true; applyPrefs(); } },
    'hr',
    { label: i.read_at ? 'Mark as unread' : 'Mark as read', icon: 'eye', onClick: () => setRead(!i.read_at) },
    { label: i.emailed_at ? `Email again · sent ${fmtTime(new Date(i.emailed_at))}` : 'Email to me', icon: 'mail', onClick: sendEmail },
  ]) }, 'Aa');

  fill(S.article, 
    h('header.masthead',
      h('div.readctl', aa),
      h('div.date', fmtFull(d)),
      h('h1', i.title),
      h('div.facts',
        h('span', `${i.minutes} min read`),
        i.sections.length ? h('span', `${i.sections.length} sections`) : null,
        h('span', i.created_by !== 'you' ? `✦ ${i.created_by} · ${fmtTime(published)}` : fmtTime(published)))),
    i.edition === iso(today()) ? S.yourday : null,
    prose,
    end,
    h('nav.pager',
      i.prev ? h('a.prev', { href: `/news/${i.prev}` }, h('small', '← Previous'), fmtMD(parseISO(i.prev))) : h('span'),
      i.next ? h('a.next', { href: `/news/${i.next}` }, h('small', 'Next →'), fmtMD(parseISO(i.next))) : null));
  applyPrefs();
  if (i.edition === iso(today())) loadDay();

  fill(S.toc, ...(i.sections.length ? [h('h4', 'In this edition'),
    ...i.sections.map((name, n) => h('a', { href: `#s${n + 1}`, onclick: (e) => { e.preventDefault(); $(`#s${n + 1}`).scrollIntoView({ behavior: 'smooth' }); } }, name))] : []));

  S.observers.forEach((o) => o.disconnect());
  S.observers = [];
  if (!i.read_at) {
    const o = new IntersectionObserver((es) => {
      if (es.some((x) => x.isIntersecting)) { o.disconnect(); setRead(true, true); }
    }, { threshold: 1 });
    o.observe(end);
    S.observers.push(o);
  }
  onScroll();
}

// ── Front page: "Your day" ──
// Today's edition opens with your own desk: what's due, what's coming, what you finished. It's
// drawn live from the overview (not baked into the Markdown), so it stays true all day.

const refreshDay = debounce(() => { if (S?.yourday.isConnected) loadDay(); }, 250);

async function loadDay() {
  try {
    const o = await get('/api/overview');
    if (S) renderDay(o);
  } catch { /* the edition still reads fine without it */ }
}

function renderDay(o) {
  const desk = [...o.tasks.overdue, ...o.tasks.today];
  const shown = desk.slice(0, 6);
  const horizon = o.countdowns.filter((c) => c.days_until >= 0 && c.days_until <= 45).slice(0, 3);
  const y = o.done_yesterday;

  const taskEl = (t) => {
    const late = t.due < o.today;
    const el = h('li', { class: late ? 'late' : '' },
      h('button.tick', { 'aria-label': `Complete “${t.title}”`, onclick: async () => {
        el.classList.add('done');
        try { await post(`/api/tasks/${t.id}/complete`); ctx.refreshNav(); setTimeout(loadDay, 450); }
        catch (e) { el.classList.remove('done'); fail(e); }
      } }, icon('check')),
      h('span.what', t.title, t.starred ? h('span.star', ' ★') : null),
      h('span.meta', late ? relDay(t.due) : t.countdown ? `${t.countdown}` : t.tags[0] ? `#${t.tags[0]}` : ''));
    return el;
  };

  fill(S.yourday,
    h('header', h('span.kicker', 'Your day'), h('span.rule'),
      h('span.tally', desk.length ? `${desk.length} on the desk` : 'Clear desk')),
    h('div.cols',
      h('div.col',
        h('h5', 'On your desk'),
        shown.length ? h('ul.desk', shown.map(taskEl)) : h('p.calm', 'Nothing due today. The page is yours.'),
        desk.length > shown.length ? h('a.more', { href: '/tasks' }, `+ ${desk.length - shown.length} more in Tasks`) : null),
      h('div.col',
        h('h5', 'On the horizon'),
        horizon.length ? h('ul.horizon-mini', horizon.map((c) => h('li',
          h('span.n', c.days_until === 0 ? 'Today' : String(c.days_until), c.days_until ? h('small', c.days_until === 1 ? 'day' : 'days') : null),
          h('span.what', c.title)))) : h('p.calm', 'No dates in the next six weeks.'))),
    y.count ? h('p.yesterday', h('b', 'Yesterday'), ` you finished ${y.count}: `,
      y.titles.join(' · '), y.count > y.titles.length ? ` and ${y.count - y.titles.length} more` : '', '.') : null);
}

async function sendEmail() {
  const i = S.issue;
  try {
    const r = await post(`/api/issues/${i.edition}/email`);
    i.emailed_at = r.emailed_at;
    toast('Emailed to you');
  } catch (e) { fail(e); }
}

async function setRead(read, quiet = false) {
  const i = S.issue;
  try {
    await put(`/api/issues/${i.edition}/read`, { read });
    i.read_at = read ? new Date().toISOString() : null;
    const s = S.issues.find((x) => x.edition === i.edition);
    if (s) s.read = read;
    renderShelf();
    ctx.refreshNav();
    if (!quiet) toast(read ? 'Marked read' : 'Marked unread');
  } catch (e) { fail(e); }
}

function onScroll() {
  if (!S?.issue) return;
  const max = Math.max(1, document.documentElement.scrollHeight - innerHeight);
  const p = Math.min(1, Math.max(0, scrollY / max));
  S.progress.style.width = `${(p * 100).toFixed(2)}%`;
  const hs = $$('.prose h2', S.article);
  let active = -1;
  hs.forEach((el, n) => { if (el.getBoundingClientRect().top < innerHeight * 0.3) active = n; });
  if (p > 0.995 && hs.length) active = hs.length - 1; // at the very end, the last section is current
  $$('a', S.toc).forEach((el, n) => el.classList.toggle('on', n === active));
}

function renderEmpty() {
  S.shelf.replaceChildren(); S.toc.replaceChildren();
  fill(S.article, h('div.empty',
    h('div.art', '¶'),
    h('h3', 'No editions yet'),
    h('p', 'Your morning news job publishes here. Connect it once in Settings, then have it call the ', h('code', 'publish_issue'), ' tool instead of sending an email.'),
    h('p', { style: { marginTop: '16px' } }, h('a.btn', { href: '/settings#ai' }, icon('link'), 'Connect an AI'))));
}
