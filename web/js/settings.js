// Settings: appearance, notifications, AI connections, iPhone capture, passkeys, signed-in devices.

import { h, icon, get, post, patch, put, del, toast, fail, sheet, copyText, store, ago, $, fill } from './util.js';
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
  const [keys, sessions, grants, pushInfo] = await Promise.all([get('/api/passkeys'), get('/api/sessions'), get('/api/connections'), get('/api/push')]).catch((e) => { fail(e); return [[], [], [], null]; });
  if (!S) return;
  const here = await currentSubscription();
  if (!S) return;
  fill(S.body, appearance(), notifications(pushInfo, here), connections(grants), capture(), passkeys(keys), devices(sessions), account());
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

// ── Notifications ──

const canPush = () => 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;
const isIOS = () => /iP(hone|ad|od)/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const standalone = () => matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;

async function currentSubscription() {
  if (!canPush()) return null;
  // `ready` never settles if the worker failed to register; don't let that hold up Settings.
  const reg = await Promise.race([navigator.serviceWorker.ready, new Promise((r) => setTimeout(() => r(null), 1500))]);
  try { return reg ? await reg.pushManager.getSubscription() : null; } catch { return null; }
}

function deviceLabel() {
  const ua = navigator.userAgent;
  const os = /iPhone/.test(ua) ? 'iPhone' : /iPad/.test(ua) || isIOS() ? 'iPad' : /Android/.test(ua) ? 'Android' : /Mac/.test(ua) ? 'Mac' : /Windows/.test(ua) ? 'Windows' : 'Linux';
  const br = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : 'Safari';
  return standalone() ? `EOF app on ${os}` : `${br} on ${os}`;
}

const b64ToBytes = (s) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (s.length % 4)) % 4)), (c) => c.charCodeAt(0));

async function turnOn(info, btn) {
  btn.disabled = true;
  try {
    if (await Notification.requestPermission() !== 'granted') {
      toast('Notifications are blocked for EOF — allow them in this browser’s settings.', { error: true });
      return;
    }
    const reg = await navigator.serviceWorker.ready;
    let sub = await reg.pushManager.getSubscription();
    if (!sub) sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64ToBytes(info.public_key) });
    await post('/api/push/subscribe', { ...sub.toJSON(), label: deviceLabel() });
    toast('Notifications on for this device');
  } catch (e) { fail(e); } finally { btn.disabled = false; render(); }
}

async function turnOff(sub) {
  try {
    await post('/api/push/unsubscribe', { endpoint: sub.endpoint });
    await sub.unsubscribe();
    toast('Notifications off for this device');
  } catch (e) { fail(e); }
  render();
}

function notifications(info, here) {
  if (!info) return null;
  let thisDevice;
  if (canPush()) {
    const blocked = !here && Notification.permission === 'denied';
    const btn = h('button.btn', here ? { onclick: () => turnOff(here) } : { class: 'primary', disabled: blocked, onclick: (e) => turnOn(info, e.currentTarget) },
      icon(here ? 'x' : 'bell'), here ? 'Turn off on this device' : 'Turn on for this device');
    thisDevice = h('div.notify-here', btn,
      blocked ? h('span.muted', 'Blocked in this browser’s settings — allow notifications for this site to turn them on.') : null);
  } else if (isIOS() && !standalone()) {
    thisDevice = h('div.install-hint',
      h('strong', 'On iPhone, notifications need the app on your Home Screen.'),
      h('ol', h('li', 'Tap ', h('b', 'Share'), ' in Safari’s toolbar.'), h('li', 'Choose ', h('b', 'Add to Home Screen'), '.'), h('li', 'Open EOF from the Home Screen and come back here.')));
  } else {
    thisDevice = h('p.muted', { style: { margin: 0 } }, 'This browser can’t receive notifications.');
  }

  const hour = h('select.input', { 'aria-label': 'Morning summary time', style: { width: 'auto' }, onchange: async (e) => {
    try { await put('/api/push/digest', { hour: e.target.value === '' ? null : Number(e.target.value) }); toast('Saved'); } catch (x) { fail(x); }
  } },
  h('option', { value: '', selected: info.digest_hour == null }, 'Off'),
  Array.from({ length: 24 }, (_, i) => h('option', { value: i, selected: info.digest_hour === i },
    new Date(2000, 0, 1, i).toLocaleTimeString(undefined, { hour: 'numeric' }))));

  return h('section.card', { id: 'notifications' },
    h('h2', 'Notifications'),
    h('p.lead', 'A short morning summary of what’s due, and a nudge when today’s edition lands.'),
    thisDevice,
    h('div.ctrl-row', icon('sun'), h('span', 'Morning summary at'), hour),
    info.devices.length ? h('div.rows', { style: { marginTop: '12px' } }, info.devices.map((d) => h('div.r',
      icon('bell'),
      h('div.grow', h('div.t', d.label), h('div.s', d.last_ok_at ? `Last delivered ${ago(d.last_ok_at)}` : `Added ${ago(d.created_at)}`)),
      h('button.btn.sm.ghost.danger', { onclick: async () => { try { await del(`/api/push/${d.id}`); render(); } catch (e) { fail(e); } } }, 'Remove')))) : null,
    info.devices.length ? h('button.btn.sm', { style: { marginTop: '10px' }, onclick: async (e) => {
      e.currentTarget.disabled = true;
      try { const r = await post('/api/push/test'); toast(r.sent ? `Sent to ${r.sent} device${r.sent === 1 ? '' : 's'}` : 'No device accepted it — try turning notifications off and on.', { error: !r.sent }); }
      catch (x) { fail(x); } finally { e.target.disabled = false; }
    } }, 'Send a test') : null);
}

// ── Capture from iPhone (Shortcuts) ──

function capture() {
  const url = `${ctx.boot.origin}/capture`;
  return h('section.card', { id: 'capture' },
    h('h2', 'Add from iPhone'),
    h('p.lead', 'One Shortcut lets you add tasks with Siri, the Action button, or the Share Sheet — written the same way as in the composer: “call dentist fri #health !”.'),
    h('div.copy', h('code', url), h('button.btn.sm', { onclick: () => copyText(url, 'Capture URL copied') }, icon('copy'), 'Copy')),
    h('ol.steps',
      h('li', h('button.btn.sm', { onclick: shortcutToken }, icon('key'), 'Create a Shortcut token'), ' and copy it.'),
      h('li', 'In Shortcuts, make a new shortcut with ', h('b', 'Dictate Text'), ' (or ', h('b', 'Ask for Input'), ').'),
      h('li', 'Add ', h('b', 'Get Contents of URL'), ': the URL above, method ', h('b', 'POST'), ', header ', h('code', 'Authorization'), ' = ', h('code', 'Bearer <token>'),
        ', request body ', h('b', 'JSON'), ' with ', h('code', 'text'), ' = the dictated text. Optional: ', h('code', 'list'), ' = a list name.'),
      h('li', 'Add ', h('b', 'Show Result'), ' (or ', h('b', 'Speak Text'), ') to hear what was added. Name it “Add to EOF” for “Hey Siri, add to EOF”.')),
    h('p.muted', { style: { margin: '10px 0 0', fontSize: '13px' } }, 'Tasks added this way are marked ✦ with the token’s name.'));
}

async function shortcutToken() {
  try {
    const r = await post('/api/connections', { name: 'iPhone Shortcut' });
    const curl = `curl -X POST ${ctx.boot.origin}/capture -H "Authorization: Bearer ${r.token}" -d "text=try EOF tomorrow #test"`;
    sheet((box, close) => box.append(
      h('h2', 'Your Shortcut token'),
      h('div.secret',
        h('strong', 'Copy it now — it won’t be shown again.'),
        h('div.copy', h('code', r.token), h('button.btn.sm.icon-only', { 'aria-label': 'Copy token', onclick: () => copyText(r.token, 'Token copied') }, icon('copy')))),
      h('div.field', h('span', 'Header value for “Get Contents of URL”'),
        h('div.copy', h('code', `Bearer ${r.token}`), h('button.btn.sm.icon-only', { 'aria-label': 'Copy header', onclick: () => copyText(`Bearer ${r.token}`, 'Header copied') }, icon('copy')))),
      h('div.field', h('span', 'Try it from a terminal'),
        h('div.copy', h('code', curl), h('button.btn.sm.icon-only', { 'aria-label': 'Copy command', onclick: () => copyText(curl) }, icon('copy')))),
      h('div.actions', h('button.btn.primary', { onclick: () => { close(); render(); } }, 'Done'))));
  } catch (e) { fail(e); }
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
