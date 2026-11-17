// Settings: appearance, AI connections, passkeys, signed-in devices.

import { h, icon, get, post, patch, del, toast, fail, sheet, copyText, store, ago, $, fill } from './util.js';
import { ctx, applyTheme, navigate } from './app.js';
import { registerPasskey } from './login.js';

let S = null;

export function mount(root) {
  S = { root, body: h('div.settings') };
  root.append(h('div.page-head', h('div', h('div.eyebrow', 'Settings'), h('h1', 'Preferences'))), S.body);
  render();
  return { unmount() { S = null; }, onChange(ev) { if (ev.area === 'settings') render(); } };
}

async function render() {
  const [keys, sessions, grants] = await Promise.all([get('/api/passkeys'), get('/api/sessions'), get('/api/connections')]).catch((e) => { fail(e); return [[], [], []]; });
  if (!S) return;
  fill(S.body, appearance(), connections(grants), passkeys(keys), devices(sessions), account());
  if (location.hash) $(location.hash)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

function appearance() {
  const cur = store.get('theme', 'system');
  return h('section.card',
    h('h2', 'Appearance'),
    h('p.lead', `Timezone ${ctx.boot.timezone} — set automatically from this browser.`),
    h('div.seg', ['system', 'light', 'dark'].map((t) => h('button', {
      class: cur === t ? 'on' : '',
      onclick: () => { store.set('theme', t); applyTheme(t); render(); },
    }, t[0].toUpperCase() + t.slice(1)))));
}

function connections(grants) {
  const url = ctx.boot.mcp_url;
  return h('section.card', { id: 'ai' },
    h('h2', 'AI connections'),
    h('p.lead', 'Everything in EOF is available to AI assistants over MCP. Changes they make show up here live, marked ✦.'),
    h('div.copy', h('code', url), h('button.btn.sm', { onclick: () => copyText(url, 'MCP URL copied') }, icon('copy'), 'Copy')),
    h('div.howto',
      h('div', h('strong', 'Claude (web, desktop, mobile)'), h('ol',
        h('li', 'Settings → Connectors → Add custom connector'),
        h('li', 'Paste the URL above. You’ll be sent here to approve it with your passkey.'))),
      h('div', h('strong', 'Claude Code'), h('div.copy', { style: { marginTop: '6px' } },
        h('code', `claude mcp add --transport http eof ${url}`),
        h('button.btn.sm.icon-only', { 'aria-label': 'Copy command', onclick: () => copyText(`claude mcp add --transport http eof ${url}`) }, icon('copy'))))),
    h('div.rows', { style: { marginTop: '16px' } },
      grants.length ? grants.map((g) => h('div.r',
        h('span.spark', '✦'),
        h('div.grow', h('div.t', g.name, ' ', h('span.badge', g.kind === 'oauth' ? 'Connected app' : 'Token')),
          h('div.s', g.last_used_at ? `Last active ${ago(g.last_used_at)}` : `Added ${ago(g.created_at)} · not used yet`)),
        h('button.btn.sm.ghost.danger', { onclick: () => confirmRevoke(g) }, 'Disconnect'))) : h('div.r', h('div.s.muted', 'Nothing connected yet.'))),
    h('div', { style: { marginTop: '12px', display: 'flex', gap: '8px', alignItems: 'center', flexWrap: 'wrap' } },
      h('button.btn', { onclick: newToken }, icon('key'), 'Create a token'),
      h('span.muted', { style: { fontSize: '13px' } }, 'For scripts and scheduled jobs that can’t do a browser sign-in.')));
}

function confirmRevoke(g) {
  sheet((box, close) => box.append(
    h('h2', `Disconnect ${g.name}?`),
    h('p.muted', { style: { margin: 0 } }, 'It will immediately lose access to EOF.'),
    h('div.actions', h('button.btn.ghost', { onclick: close }, 'Cancel'),
      h('button.btn.primary', { onclick: async () => { try { await del(`/api/connections/${g.id}`); close(); render(); toast(`Disconnected ${g.name}`); } catch (e) { fail(e); } } }, 'Disconnect'))));
}

function newToken() {
  sheet((box, close) => {
    const name = h('input.input', { placeholder: 'e.g. Morning news job', maxLength: 40 });
    const go = async () => {
      if (!name.value.trim()) return name.focus();
      try {
        const r = await post('/api/connections', { name: name.value.trim() });
        const cmd = `claude mcp add --transport http eof ${ctx.boot.mcp_url} --header "Authorization: Bearer ${r.token}"`;
        fill(box, 
          h('h2', 'Your token'),
          h('div.secret',
            h('strong', 'Copy it now — it won’t be shown again.'),
            h('div.copy', h('code', r.token), h('button.btn.sm.icon-only', { 'aria-label': 'Copy token', onclick: () => copyText(r.token, 'Token copied') }, icon('copy')))),
          h('div.field', h('span', 'Use with Claude Code'),
            h('div.copy', h('code', cmd), h('button.btn.sm.icon-only', { 'aria-label': 'Copy command', onclick: () => copyText(cmd) }, icon('copy')))),
          h('p.muted', { style: { margin: 0, fontSize: '13px' } }, 'Any MCP client works: send it as ', h('code', 'Authorization: Bearer …'), ' to the MCP URL.'),
          h('div.actions', h('button.btn.primary', { onclick: () => { close(); render(); } }, 'Done')));
      } catch (e) { fail(e); }
    };
    name.addEventListener('keydown', (e) => e.key === 'Enter' && go());
    box.append(h('h2', 'Create a token'), h('p.muted', { style: { margin: 0, fontSize: '14px' } }, 'Name it after what will use it. Its name appears on everything it creates.'),
      name, h('div.actions', h('button.btn.ghost', { onclick: close }, 'Cancel'), h('button.btn.primary', { onclick: go }, 'Create')));
  });
}

function passkeys(keys) {
  return h('section.card',
    h('h2', 'Passkeys'),
    h('p.lead', 'EOF has no password. Add a passkey on each device you use — or keep one on your phone.'),
    h('div.rows', keys.map((k) => h('div.r',
      icon('key'),
      h('div.grow', h('div.t', k.name), h('div.s', `Added ${ago(k.created_at)}${k.last_used_at ? ` · used ${ago(k.last_used_at)}` : ''}`)),
      h('button.btn.sm.ghost', { onclick: () => rename(k) }, 'Rename'),
      keys.length > 1 ? h('button.btn.sm.ghost.danger', { onclick: () => removeKey(k) }, 'Remove') : null))),
    h('button.btn', { style: { marginTop: '10px' }, onclick: async (e) => {
      e.currentTarget.disabled = true;
      try { const r = await registerPasskey({}); toast(`Added passkey “${r.name}”`); render(); }
      catch (x) { fail(x.name === 'NotAllowedError' ? 'Cancelled' : x); e.target.disabled = false; }
    } }, icon('plus'), 'Add a passkey'));
}

function rename(k) {
  sheet((box, close) => {
    const input = h('input.input', { value: k.name });
    const go = async () => { try { await patch(`/api/passkeys/${k.id}`, { name: input.value }); close(); render(); } catch (e) { fail(e); } };
    input.addEventListener('keydown', (e) => e.key === 'Enter' && go());
    box.append(h('h2', 'Rename passkey'), input, h('div.actions', h('button.btn.ghost', { onclick: close }, 'Cancel'), h('button.btn.primary', { onclick: go }, 'Save')));
  });
}

async function removeKey(k) {
  try { await del(`/api/passkeys/${k.id}`); toast(`Removed “${k.name}”`); render(); } catch (e) { fail(e); }
}

function devices(sessions) {
  return h('section.card',
    h('h2', 'Signed-in devices'),
    h('div.rows', sessions.map((s) => h('div.r',
      h('div.grow', h('div.t', s.label, s.current ? h('span.badge', { style: { marginLeft: '8px' } }, 'This device') : null),
        h('div.s', `Signed in ${ago(s.created_at)} · active ${ago(s.last_seen_at)}`)),
      s.current ? null : h('button.btn.sm.ghost.danger', { onclick: async () => { try { await del(`/api/sessions/${s.id}`); render(); } catch (e) { fail(e); } } }, 'Sign out')))));
}

function account() {
  return h('section', { style: { display: 'flex', justifyContent: 'center' } },
    h('button.btn.ghost', { onclick: async () => { await post('/api/logout').catch(() => {}); ctx.boot = null; navigate('/login'); } }, icon('logout'), 'Sign out'));
}
