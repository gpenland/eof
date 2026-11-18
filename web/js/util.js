// Small DOM + data toolkit. No framework: pages render with h() and re-render on change.

/** h('div.a.b', {onclick, class, style, ...attrs}, ...children) */
export function h(sel, props, ...kids) {
  if (props == null || typeof props !== 'object' || props instanceof Node || Array.isArray(props)) {
    if (props != null) kids.unshift(props);
    props = {};
  }
  const [tag, ...classes] = sel.split('.');
  const el = document.createElement(tag || 'div');
  if (classes.length) el.className = classes.join(' ');
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'class') el.className += (el.className ? ' ' : '') + v;
    else if (k === 'style' && typeof v === 'object') {
      for (const [sk, sv] of Object.entries(v)) {
        if (sk.startsWith('--')) el.style.setProperty(sk, sv); // custom properties need setProperty
        else el.style[sk] = sv;
      }
    }
    else if (k === 'html') el.innerHTML = v; // only ever used with server-sanitized HTML or our own icons
    else if (k in el && k !== 'list' && k !== 'form') el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  append(el, kids);
  return el;
}

function append(el, kids) {
  for (const k of kids.flat(Infinity)) {
    if (k == null || k === false) continue;
    el.append(k instanceof Node ? k : document.createTextNode(String(k)));
  }
}

/** replaceChildren that skips null/false, like h() does. */
export function fill(el, ...kids) {
  el.replaceChildren();
  append(el, kids);
  return el;
}

export const $ = (s, root = document) => root.querySelector(s);
export const $$ = (s, root = document) => [...root.querySelectorAll(s)];

// ── Icons (Lucide-style strokes, drawn inline) ──
const P = {
  check: '<path d="M20 6 9 17l-5-5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  star: '<path d="m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  repeat: '<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>',
  tasks: '<circle cx="5" cy="6" r="1.5"/><circle cx="5" cy="12" r="1.5"/><circle cx="5" cy="18" r="1.5"/><path d="M10 6h10M10 12h10M10 18h7"/>',
  hourglass: '<path d="M6 2h12M6 22h12M7 2v4a5 5 0 0 0 10 0V2M7 22v-4a5 5 0 0 1 10 0v4"/>',
  paper: '<path d="M4 4h13a3 3 0 0 1 3 3v13H7a3 3 0 0 1-3-3z"/><path d="M8 8h8M8 12h8M8 16h5"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
  right: '<path d="m9 6 6 6-6 6"/>',
  left: '<path d="m15 6-6 6 6 6"/>',
  dots: '<circle cx="12" cy="5" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="12" cy="19" r="1"/>',
  grip: '<circle cx="9" cy="6" r="1"/><circle cx="15" cy="6" r="1"/><circle cx="9" cy="12" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="9" cy="18" r="1"/><circle cx="15" cy="18" r="1"/>',
  x: '<path d="M18 6 6 18M6 6l12 12"/>',
  trash: '<path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14"/>',
  key: '<circle cx="7.5" cy="15.5" r="4.5"/><path d="m10.7 12.3 9.8-9.8M17 6l3 3M14 9l2 2"/>',
  fingerprint: '<path d="M12 10a2 2 0 0 0-2 2c0 1.02-.1 2.51-.26 4"/><path d="M14 13.12c0 2.38 0 6.38-1 8.88"/><path d="M17.29 21.02c.12-.6.43-2.3.5-3.02"/><path d="M2 12a10 10 0 0 1 18-6"/><path d="M2 16h.01"/><path d="M21.8 16c.2-2 .131-5.354 0-6"/><path d="M5 19.5C5.5 18 6 15 6 12a6 6 0 0 1 .34-2"/><path d="M8.65 22c.21-.66.45-1.32.57-2"/><path d="M9 6.8a6 6 0 0 1 9 5.2v2"/>',
  copy: '<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  tag: '<path d="M20.6 13.4 13.4 20.6a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8z"/><circle cx="7.5" cy="7.5" r="1.5"/>',
  cal: '<rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/>',
  type: '<path d="M4 7V4h16v3M9 20h6M12 4v16"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
  shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>',
  mail: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/>',
  link: '<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/>',
  logout: '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/>',
  list: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
  bell: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
};
export function icon(name, cls = '') {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 24 24');
  s.setAttribute('class', `icon ${cls}`.trim());
  s.setAttribute('aria-hidden', 'true');
  s.innerHTML = P[name] || '';
  return s;
}

// ── API ──
export class ApiError extends Error {
  constructor(status, msg) { super(msg); this.status = status; }
}
export async function api(method, path, body) {
  const opts = { method, headers: { 'x-eof': '1' }, credentials: 'same-origin' };
  if (body !== undefined) {
    opts.headers['content-type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  const res = await fetch(path, opts);
  let data = null;
  try { data = await res.json(); } catch { /* empty body */ }
  if (res.status === 401 && path.startsWith('/api/')) {
    window.dispatchEvent(new CustomEvent('eof:signed-out'));
  }
  if (!res.ok) throw new ApiError(res.status, data?.error || `Request failed (${res.status})`);
  return data;
}
export const get = (p) => api('GET', p);
export const post = (p, b = {}) => api('POST', p, b);
export const patch = (p, b) => api('PATCH', p, b);
export const put = (p, b) => api('PUT', p, b);
export const del = (p) => api('DELETE', p);

// ── Dates (all local time; ISO strings are YYYY-MM-DD) ──
export const pad = (n) => String(n).padStart(2, '0');
export const iso = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
export const parseISO = (s) => { const [y, m, d] = s.split('-').map(Number); return new Date(y, m - 1, d); };
export const addDays = (d, n) => { const x = new Date(d); x.setDate(x.getDate() + n); return x; };
export const today = () => { const d = new Date(); d.setHours(0, 0, 0, 0); return d; };
export const dayDiff = (a, b) => Math.round((parseISO(iso(a)) - parseISO(iso(b))) / 864e5);
const fmt = (o) => new Intl.DateTimeFormat(undefined, o);
export const fmtShort = (d) => fmt({ weekday: 'short', month: 'short', day: 'numeric' }).format(d);
export const fmtLong = (d) => fmt({ month: 'long', day: 'numeric', year: 'numeric' }).format(d);
export const fmtFull = (d) => fmt({ weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' }).format(d);
export const fmtMD = (d) => fmt({ month: 'short', day: 'numeric' }).format(d);
export const fmtWd = (d) => fmt({ weekday: 'short' }).format(d);
export const fmtTime = (d) => fmt({ hour: 'numeric', minute: '2-digit' }).format(d);
/** "T-14", "day of", "+2" — a countdown milestone's distance from its date. */
export const milestoneLabel = (n) => (n > 0 ? `T-${n}` : n === 0 ? 'day of' : `+${-n}`);

export function relDay(isoDate) {
  const n = dayDiff(parseISO(isoDate), today());
  if (n === 0) return 'Today';
  if (n === 1) return 'Tomorrow';
  if (n === -1) return 'Yesterday';
  const d = parseISO(isoDate);
  if (n > 1 && n < 7) return fmtWd(d);
  return d.getFullYear() === today().getFullYear() ? fmtShort(d) : fmtLong(d);
}
export function ago(ts) {
  const s = (Date.now() - new Date(ts)) / 1000;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d ago`;
  return fmtLong(new Date(ts));
}

// ── Feedback ──
export function toast(msg, { action, onAction, ai = false, error = false, ms = 4200 } = {}) {
  const box = document.getElementById('toasts');
  const el = h('div.toast', { class: error ? 'err' : '' },
    ai ? h('span.spark', '✦') : null,
    h('span', msg),
    action ? h('button', { onclick: () => { onAction?.(); close(); } }, action) : null);
  box.append(el);
  while (box.children.length > 3) box.firstChild.remove();
  let t = setTimeout(close, ms);
  el.addEventListener('mouseenter', () => clearTimeout(t));
  el.addEventListener('mouseleave', () => { t = setTimeout(close, 1800); });
  function close() { el.classList.add('out'); setTimeout(() => el.remove(), 200); }
  return close;
}
export const fail = (e) => toast(e?.message || String(e), { error: true });

/** Anchored popup menu. items: [{label, icon, onClick, check, danger} | 'hr' | {section}] */
export function menu(anchor, items) {
  closeMenus();
  const el = h('div.menu', { role: 'menu' }, items.map((it) => {
    if (it === 'hr') return h('hr');
    if (it.section) return h('div.label', it.section);
    return h('button', {
      role: 'menuitem', class: it.danger ? 'btn danger' : '', style: it.danger ? { color: 'var(--late)', border: 0, height: 'auto', justifyContent: 'flex-start', fontWeight: 400 } : null,
      onclick: (e) => { e.stopPropagation(); closeMenus(); it.onClick?.(); },
    }, it.icon ? icon(it.icon) : null, it.label, it.check ? h('span.check', icon('check')) : null);
  }));
  document.body.append(el);
  const r = anchor.getBoundingClientRect();
  const w = el.offsetWidth, hgt = el.offsetHeight;
  el.style.left = Math.max(8, Math.min(r.left, innerWidth - w - 8)) + 'px';
  el.style.top = (r.bottom + 6 + hgt > innerHeight ? Math.max(8, r.top - hgt - 6) : r.bottom + 6) + 'px';
  setTimeout(() => document.addEventListener('click', closeMenus, { once: true }));
  el.querySelector('button')?.focus();
  el.addEventListener('keydown', (e) => {
    const bs = $$('button', el); const i = bs.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') { e.preventDefault(); bs[(i + 1) % bs.length].focus(); }
    if (e.key === 'ArrowUp') { e.preventDefault(); bs[(i - 1 + bs.length) % bs.length].focus(); }
    if (e.key === 'Escape') { closeMenus(); anchor.focus?.(); }
  });
  return el;
}
export function closeMenus() { $$('.menu').forEach((m) => m.remove()); }

/** Modal sheet; returns {el, close}. */
export function sheet(build) {
  const prev = document.activeElement;
  const back = h('div.sheet-backdrop', { onmousedown: (e) => { if (e.target === back) close(); } });
  const box = h('div.sheet', { role: 'dialog', 'aria-modal': 'true' });
  back.append(box);
  function close() { back.remove(); document.removeEventListener('keydown', esc, true); prev?.focus?.(); }
  function esc(e) { if (e.key === 'Escape') { e.stopPropagation(); close(); } }
  document.addEventListener('keydown', esc, true);
  build(box, close);
  document.body.append(back);
  setTimeout(() => box.querySelector('input,textarea,button')?.focus(), 20);
  return { el: box, close };
}

export function debounce(fn, ms) {
  let t;
  const d = (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
  d.flush = (...a) => { clearTimeout(t); fn(...a); };
  return d;
}

export function autosize(ta) {
  const fit = () => { ta.style.height = 'auto'; ta.style.height = ta.scrollHeight + 'px'; };
  ta.addEventListener('input', fit);
  requestAnimationFrame(fit);
  return ta;
}

export async function copyText(text, label = 'Copied') {
  try { await navigator.clipboard.writeText(text); toast(label); } catch { fail('Copy failed — select and copy manually'); }
}

export const isTyping = (e) => {
  const t = e.target;
  return t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName);
};

export const store = {
  get(k, d) { try { const v = localStorage.getItem('eof:' + k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem('eof:' + k, JSON.stringify(v)); } catch { /* private mode */ } },
};

// ── WebAuthn JSON <-> ArrayBuffer ──
export const b64u = {
  dec(s) {
    s = s.replace(/-/g, '+').replace(/_/g, '/');
    const bin = atob(s + '='.repeat((4 - (s.length % 4)) % 4));
    return Uint8Array.from(bin, (c) => c.charCodeAt(0)).buffer;
  },
  enc(buf) {
    const b = new Uint8Array(buf); let s = '';
    for (const x of b) s += String.fromCharCode(x);
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  },
};
