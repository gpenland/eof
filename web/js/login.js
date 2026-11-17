// Lock screen (passkeys), first-run setup, and the OAuth consent screen for connecting an AI.

import { h, icon, get, post, b64u, fmtFull, fill } from './util.js';

// ── WebAuthn ──

function toCreate(o) {
  const pk = structuredClone(o.publicKey);
  pk.challenge = b64u.dec(pk.challenge);
  pk.user.id = b64u.dec(pk.user.id);
  pk.excludeCredentials = (pk.excludeCredentials || []).map((c) => ({ ...c, id: b64u.dec(c.id) }));
  return { publicKey: pk };
}
function toGet(o) {
  const pk = structuredClone(o.publicKey);
  pk.challenge = b64u.dec(pk.challenge);
  pk.allowCredentials = (pk.allowCredentials || []).map((c) => ({ ...c, id: b64u.dec(c.id) }));
  return { publicKey: pk };
}

export async function registerPasskey({ name, setup_code } = {}) {
  if (!window.PublicKeyCredential) throw new Error('This browser doesn’t support passkeys.');
  const { ceremony, options } = await post('/auth/register/start', { name, setup_code });
  const cred = await navigator.credentials.create(toCreate(options));
  const r = cred.response;
  return post('/auth/register/finish', {
    ceremony,
    credential: {
      id: cred.id, rawId: b64u.enc(cred.rawId), type: cred.type,
      response: { attestationObject: b64u.enc(r.attestationObject), clientDataJSON: b64u.enc(r.clientDataJSON), transports: r.getTransports?.() ?? [] },
      extensions: cred.getClientExtensionResults?.() ?? {},
    },
  });
}

async function signIn() {
  const { ceremony, options } = await post('/auth/login/start');
  const cred = await navigator.credentials.get(toGet(options));
  const r = cred.response;
  return post('/auth/login/finish', {
    ceremony,
    credential: {
      id: cred.id, rawId: b64u.enc(cred.rawId), type: cred.type,
      response: {
        authenticatorData: b64u.enc(r.authenticatorData), clientDataJSON: b64u.enc(r.clientDataJSON),
        signature: b64u.enc(r.signature), userHandle: r.userHandle ? b64u.enc(r.userHandle) : null,
      },
      extensions: cred.getClientExtensionResults?.() ?? {},
    },
  });
}

const friendly = (e) => (e?.name === 'NotAllowedError' ? 'Cancelled — or the passkey prompt timed out.' : e?.name === 'InvalidStateError' ? 'This device already has a passkey here.' : e?.message || String(e));

// ── Lock screen ──

export function lockScreen(status, done) {
  const clock = h('div.clock');
  const date = h('div.today');
  const tickClock = () => {
    const d = new Date();
    clock.textContent = d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }).replace(/\s?[AP]M$/i, '');
    date.textContent = fmtFull(d);
  };
  tickClock();
  const timer = setInterval(() => { if (!clock.isConnected) clearInterval(timer); else tickClock(); }, 1000);
  const err = h('div.err', { role: 'alert' });
  const panel = h('div.panel', h('div.wordmark', 'EOF', h('span.caret')), h('div', clock, date));

  if (status.needs_setup) {
    const code = h('input.input.code', { placeholder: 'XXXX-XXXX-XXXX', autocomplete: 'one-time-code', spellcheck: false, 'aria-label': 'Setup code', maxLength: 80 });
    code.addEventListener('input', () => {
      const raw = code.value.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 64);
      code.value = raw.match(/.{1,4}/g)?.join('-') ?? '';
    });
    const name = h('input.input', { placeholder: 'This device, e.g. “MacBook”', 'aria-label': 'Passkey name' });
    const btn = h('button.btn.primary', { type: 'submit' }, icon('fingerprint', 'fingerprint'), 'Create passkey');
    const form = h('form', { onsubmit: async (e) => {
      e.preventDefault(); err.textContent = ''; btn.disabled = true;
      try { await registerPasskey({ name: name.value, setup_code: code.value }); done(); } catch (x) { err.textContent = friendly(x); } finally { btn.disabled = false; }
    } },
    h('label.field', h('span', 'Setup code'), code),
    h('label.field', h('span', 'Name this passkey'), name),
    btn);
    panel.append(
      h('p.fine', h('strong', 'Welcome. '), 'EOF has no passwords. Enter the one-time setup code printed in your server’s log, then save a passkey with Touch ID, Face ID, Windows Hello or your phone.'),
      form, err);
    setTimeout(() => code.focus(), 50);
  } else {
    const btn = h('button.btn.primary', { onclick: go }, icon('fingerprint', 'fingerprint'), 'Unlock with passkey');
    async function go() {
      err.textContent = ''; btn.disabled = true;
      try { await signIn(); done(); } catch (x) { err.textContent = friendly(x); } finally { btn.disabled = false; }
    }
    panel.append(btn, err, h('p.fine', 'Private. Use the passkey saved on your device or phone.'));
    document.addEventListener('keydown', function enter(e) {
      if (!btn.isConnected) return document.removeEventListener('keydown', enter);
      if (e.key === 'Enter' && document.activeElement === document.body) go();
    });
    setTimeout(() => btn.focus(), 50);
  }
  return h('div.lock', panel);
}

// ── OAuth consent ──

export function consentScreen() {
  const params = Object.fromEntries(new URLSearchParams(location.search));
  const card = h('div.consent', h('p.muted', 'Loading…'));
  const wrap = h('div.lock', card);
  (async () => {
    try {
      const c = await get(`/api/oauth/client/${encodeURIComponent(params.client_id || '')}`);
      const decide = async (allow, btn) => {
        btn.disabled = true;
        try { const r = await post('/api/oauth/approve', { ...params, allow }); location.href = r.redirect; }
        catch (e) { btn.disabled = false; card.append(h('div.err', e.message)); }
      };
      const allowBtn = h('button.btn.primary', { onclick: (e) => decide(true, e.currentTarget) }, 'Allow access');
      fill(card, 
        h('div.who', h('div.av', '✦'), h('div', h('div.eyebrow', 'Connect'), h('h1', `${c.name} wants to use EOF`))),
        h('ul',
          h('li', icon('tasks'), h('span', 'Read, add, edit and complete your tasks')),
          h('li', icon('hourglass'), h('span', 'Manage your countdown dates')),
          h('li', icon('paper'), h('span', 'Publish and read your news editions')),
          h('li', icon('shield'), h('span', `Returns you to ${c.redirect_hosts.join(', ') || 'the app'}. You can disconnect it any time in Settings.`))),
        h('div.actions', h('button.btn', { onclick: (e) => decide(false, e.currentTarget) }, 'Deny'), allowBtn));
      allowBtn.focus();
    } catch (e) {
      fill(card, h('h1', 'Can’t connect'), h('p.muted', e.message || 'This link is invalid or expired. Start the connection again from your AI app.'));
    }
  })();
  return wrap;
}
