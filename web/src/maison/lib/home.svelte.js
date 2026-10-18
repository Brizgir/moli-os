// The family dashboard's model: the hub's live state (SSE, shared with the
// Atelier), the home layout (/api/home), and gentle helpers on top.

import { hub, connect, login, decide, refreshSession, onHomeChanged } from '../../lib/hub.svelte.js';
import { t, locale } from '../../lib/i18n.svelte.js';
import { justified, visiblePages, camerasOnlyRing, heatedRoom, ringing, PAGE_IDS, PAGE_CHOICES } from './pages.js';

export { hub, justified, visiblePages, heatedRoom, ringing, PAGE_IDS, PAGE_CHOICES };

export const home = $state({
  /** @type {any} layout from /api/home ({} = automatic) */
  config: null,
  /** an order the guard held, waiting for « c'est moi » */
  held: null,
  /** the installation, put off for now (a house already in use) */
  setupLater: false,
  /** short messages (« Envoi… », errors) */
  notes: [],
  /** theme preference: 'auto' follows daylight */
  theme: readPref('maison-theme', 'auto'),
  now: Date.now(),
  /** `/api/plan` (its floors say whether the house has a plan) */
  plan: null,
  /** `/api/energy`: `{ available, meters }` (404: no `[energy]`) */
  energy: null,
});

function readPref(key, fallback) {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export function savePref(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* private mode: keep it for this visit */
  }
}

let started = false;

/** Starts the live link (once) and loads the layout. */
export async function start() {
  if (started) return;
  started = true;
  connect();
  setInterval(() => (home.now = Date.now()), 15_000);
  // Written from another screen (a room declared): read again at once.
  onHomeChanged(loadLayout);
  loadFacts();
  await loadLayout();
}

/** What the menu reads once to know the house: its plan, its meters. */
async function loadFacts() {
  try {
    const res = await fetch('/api/plan');
    home.plan = res.ok ? await res.json() : null;
  } catch {
    home.plan = null;
  }
  try {
    const res = await fetch('/api/energy');
    const body = res.ok ? await res.json().catch(() => ({})) : {};
    home.energy = { available: res.ok, meters: body.meters ?? [] };
  } catch {
    home.energy = { available: false, meters: [] };
  }
}

/** The layout (`/api/home`); `{}` when the house has none. */
async function loadLayout() {
  try {
    const res = await fetch('/api/home');
    home.config = res.ok ? await res.json() : {};
  } catch {
    home.config = {};
  }
}

export function note(text, tone = 'info') {
  const id = Date.now() + Math.random();
  home.notes = [...home.notes, { id, text, tone }];
  setTimeout(() => (home.notes = home.notes.filter((n) => n.id !== id)), 4500);
}

// ---- reading -------------------------------------------------------------------

export const device = (id) => (id ? (hub.devices[id] ?? null) : null);

export function value(id, key) {
  const v = hub.devices[id]?.state?.[key]?.value;
  return v === undefined ? null : v;
}

export function since(id, key) {
  return hub.devices[id]?.state?.[key]?.ts ?? null;
}

// What a sensor says, whatever its brand: Zigbee (`contact` true when
// closed, `water_leak`), Tuya (`doorcontact_state` true when open,
// `watersensor_state` « alarm », `pir` « pir », `battery_state` « low »).

/** true: open, false: closed, null: not a door, no report yet, or a
 *  sensor gone silent (dead battery: its last word stays « open »). */
export function doorOpen(id) {
  if (device(id)?.online === false) return null;
  const c = value(id, 'contact');
  if (typeof c === 'boolean') return !c;
  const tuya = value(id, 'doorcontact_state');
  return typeof tuya === 'boolean' ? tuya : null;
}

export const leak = (id) => value(id, 'water_leak') === true || value(id, 'watersensor_state') === 'alarm';

export const motion = (id) => value(id, 'occupancy') === true || value(id, 'pir') === 'pir';

/** A sensor's battery (not a phone's: it charges). */
export function lowBattery(id) {
  if (device(id)?.points.some((p) => p.key === 'charging')) return false;
  if (value(id, 'battery_low') === true || value(id, 'battery_state') === 'low') return true;
  const pct = value(id, 'battery_percentage') ?? value(id, 'battery');
  return typeof pct === 'number' && pct <= 15;
}

/** Unreachable devices are said so; unknown (no report yet) counts as fine. */
export const reachable = (id) => device(id) != null && device(id).online !== false;

export function nameOf(id, fallback) {
  const d = device(id);
  return fallback || d?.label?.name || d?.native_name || t('commun.appareil');
}

/** The point that switches the device on and off. */
export function powerKey(d) {
  if (!d) return null;
  const writable = d.points.filter((p) => p.access?.write);
  const p =
    writable.find((p) => p.semantic === 'on_off' && p.kind.type === 'binary') ??
    writable.find((p) => ['on', 'switch_1', 'switch', 'state'].includes(p.key));
  return p?.key ?? null;
}

const truthy = (v) => v === true || v === 'ON' || v === 'on' || v === 1;

/** Really on, now. An unreachable device is not (a bulb cut at the wall
 *  switch keeps its last « on » on the bridge); a group (a Hue room) is on
 *  when one of its lamps that answers is (D12). */
export function isOn(id, point = null) {
  const d = device(id);
  if (!d || d.online === false) return false;
  if (!point && isGroup(d) && d.members?.length) return d.members.some((m) => m !== id && isOn(m));
  const key = point ?? powerKey(d);
  return key ? truthy(value(id, key)) : false;
}

/** A light fixture (D12): bulbs and the relay that feeds them as one light. */
export const isFixture = (d) => d?.model === 'Luminaire';
/** A Hue room or zone: « toute la pièce », not a lamp. */
export const isGroup = (d) => d?.model === 'room' || d?.model === 'zone';

/** The fixture a device belongs to (its bulbs, its relay), if any. */
export function fixtureOf(id) {
  for (const d of Object.values(hub.devices)) if (isFixture(d) && d.members?.includes(id)) return d;
  return null;
}

/** A lamp without power: cut at the wall switch (or at its relay). */
export function unpowered(id) {
  const d = device(id);
  if (!d) return false;
  if (isFixture(d)) return value(id, 'powered') === false;
  return d.online === false && ['light', 'group'].includes(deviceKind(d));
}

export function pending(id, point = null) {
  const key = point ?? powerKey(device(id));
  return key ? `${id}/${key}` in hub.pending : false;
}

// ---- rooms ---------------------------------------------------------------------

const norm = (s) =>
  String(s ?? '')
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .trim()
    .replace(/\s+/g, ' ')
    .toLowerCase();

/** The layout's name for a room called `raw` somewhere (a bridge, the plan). */
export function roomNamed(raw) {
  for (const room of home.config?.rooms ?? []) {
    if ([room.name, ...(room.aliases ?? [])].some((n) => norm(n) === norm(raw))) return room.name;
  }
  return raw;
}

/** The room a device is in, by the layout's names and aliases. */
export function roomOf(d) {
  const raw = d?.label?.room || d?.native_room;
  return raw ? roomNamed(raw) : null;
}

export const protectedRoom = (name) =>
  (hub.guard?.protected_rooms ?? []).some((p) =>
    [name, ...((home.config?.rooms ?? []).find((r) => r.name === name)?.aliases ?? [])].some(
      (n) => norm(n) === norm(p),
    ),
  );

export function hidden(id) {
  return (home.config?.hidden ?? []).includes(id);
}

/** In its room but never a button (a dishwasher, a car charger): a chip
 *  that says what it draws. `home.json` `quiet`. */
export function quiet(id) {
  return (home.config?.quiet ?? []).includes(id);
}

/** A power strip's outlets, each its own pad: `home.json` `outlets`
 *  `{ "<id>": { "switch_1": "Écran", … } }`, in that order. */
export function outletsOf(id) {
  const names = home.config?.outlets?.[id];
  const d = device(id);
  if (!names || !d) return [];
  return Object.entries(names)
    .filter(([key]) => d.points.some((p) => p.key === key && p.access?.write))
    .map(([key, name]) => ({ key, name }));
}

const LIGHTISH = /lumi|lamp|light|spot|plafon|applique|ampoule|globe|suspension|bureau|salon|cuisine|couloir|studio|véranda|veranda|entrée|chambre|toilette|combles|meuble|bulb|ceiling|desk|kitchen|hallway|bedroom|attic|fixture|lounge/i;
/** On/off things that are not lights, whatever their bridge calls them. */
const NOT_LIGHT = /vmc|ventil|pompe|prise|chauffe|\bfan\b|pump|heater|outlet|socket/i;
const has = (d, key, write = false) => d.points.some((p) => p.key === key && (!write || p.access?.write));

/** What a device is, for choosing how to show it. */
export function deviceKind(d) {
  if (!d) return 'unknown';
  if (has(d, 'progress') && has(d, 'nozzle_temperature')) return 'printer';
  if (d.camera) return 'camera';
  if (has(d, 'target_temperature')) return 'climate';
  if (has(d, 'key', true)) return 'tv';
  // A speaker sets its volume; a multi-room group (Alexa) has none, but
  // takes music by name.
  if (has(d, 'playing', true) && (has(d, 'volume') || has(d, 'play', true))) return 'speaker';
  if (has(d, 'mist_mode', true)) return 'diffuser';
  if (has(d, 'percent_control') || (has(d, 'control', true) && d.points.find((p) => p.key === 'control')?.kind.values?.includes('open')))
    return 'cover';
  if (powerKey(d)) {
    if (isGroup(d)) return 'group';
    if (isFixture(d)) return 'light';
    const name = d.label?.name || d.native_name;
    if (NOT_LIGHT.test(name) || /plug/i.test(d.model ?? '')) return 'plug';
    if (d.id.startsWith('hue:') || LIGHTISH.test(name)) return 'light';
    return 'plug';
  }
  return 'sensor';
}

/** What a speaker does: `playing`, `paused` (Alexa `paused`, Sonos
 *  `paused_playback`) or `stopped`. */
export function speakerState(id) {
  if (value(id, 'playing') === true) return 'playing';
  const state = String(value(id, 'state') ?? '');
  // An Echo none of Amazon's answers describes: not asserted stopped
  // (the person would press « play » over music that plays).
  if (value(id, 'playing') == null && state === 'unknown') return 'unknown';
  return /^paused/.test(state) ? 'paused' : 'stopped';
}

/** A multi-room group of speakers (Alexa « Partout »): no volume of its own. */
export const speakerGroup = (d) => deviceKind(d) === 'speaker' && !has(d, 'volume');

/** Speakers and groups in no room (an Alexa group is in none): shown
 *  together, « dans toute la maison ». */
export function roamingSpeakers() {
  return Object.values(hub.devices).filter((d) => !hidden(d.id) && !roomOf(d) && deviceKind(d) === 'speaker');
}

/** Rooms with their devices sorted by kind, the layout's order first. A
 *  room declared in the layout shows even with nothing in it yet
 *  (`declared`, `empty`): the place to put something. */
export function roomList() {
  const defs = home.config?.rooms ?? [];
  const map = new Map(defs.map((r) => [r.name, []]));
  // A fixture's bulbs and relay show inside it, not beside it.
  const inFixture = new Set(Object.values(hub.devices).filter(isFixture).flatMap((d) => d.members ?? []));
  for (const d of Object.values(hub.devices)) {
    if (hidden(d.id) || inFixture.has(d.id)) continue;
    const room = roomOf(d);
    if (!room) continue;
    if (!map.has(room)) map.set(room, []);
    map.get(room).push(d);
  }
  return [...map.entries()]
    .filter(([name, list]) => list.length || defs.some((r) => r.name === name))
    .map(([name, list]) => {
      const by = (...kinds) => list.filter((d) => kinds.includes(deviceKind(d)));
      const def = defs.find((r) => r.name === name);
      return {
        name,
        icon: def?.icon ?? 'home',
        declared: !!def,
        aliases: def?.aliases ?? [],
        empty: !list.length,
        group: by('group')[0] ?? null,
        lights: by('light'),
        plugs: by('plug').filter((d) => !quiet(d.id)),
        covers: by('cover'),
        climates: by('climate'),
        diffusers: by('diffuser'),
        media: by('tv', 'speaker'),
        sensors: [...by('sensor', 'camera', 'printer'), ...by('plug').filter((d) => quiet(d.id))],
        lit: by('light').filter((d) => isOn(d.id)).length,
      };
    });
}

/** The lamps of a room an ambiance can reach: those that dim, tint or warm,
 *  and the members of its group (a Hue room). */
export function lampsOf(room) {
  const can = (d) => d?.points.some((p) => ['brightness', 'color', 'color_temp'].includes(p.key) && p.access?.write);
  const ids = [];
  const add = (id) => !ids.includes(id) && ids.push(id);
  for (const d of room.lights) {
    // A fixture's bulbs take the look one by one (a colour each).
    if (isFixture(d)) (d.members ?? []).filter((m) => can(device(m))).forEach(add);
    else if (can(d)) add(d.id);
  }
  for (const m of room.group?.members ?? []) add(m);
  return ids;
}

// ---- acting --------------------------------------------------------------------

const headers = { 'content-type': 'application/json', 'x-moli-origin': 'ui' };

/** Sends an order. Held by the guard (protected room, quiet hours): asks
 *  « c'est moi » through the PIN sheet, then approves it. */
export async function act(point, val, label) {
  hub.pending[point] = val;
  try {
    const res = await fetch('/api/command', {
      method: 'POST',
      headers,
      body: JSON.stringify({ point, value: val }),
    });
    const body = await res.json().catch(() => ({}));
    if (res.status === 202) {
      delete hub.pending[point];
      home.held = { id: body.pending, point, reason: body.reason, label };
      return 'held';
    }
    if (!res.ok) throw new Error(body.error ?? res.statusText);
    setTimeout(() => {
      if (hub.pending[point] === val) delete hub.pending[point];
    }, 8000);
    return 'ok';
  } catch (err) {
    delete hub.pending[point];
    note(t('commun.echec', { label: label ?? t('commun.commande'), error: humanError(err.message) }), 'error');
    return 'error';
  }
}

// ---- rooms ---------------------------------------------------------------------

/** What a room may look like (each one is in `lib/icons.js`, and the
 *  server accepts these only). */
export const ROOM_ICONS = [
  'home', 'sofa-outline', 'bed-outline', 'bed-king-outline', 'silverware-fork-knife', 'shower-head', 'toilet',
  'desk-lamp', 'computer', 'book', 'tv', 'speaker', 'stairs', 'door', 'gate', 'car', 'package',
  'flower-outline', 'leaf', 'palm', 'pool', 'pine', 'sun', 'home-roof', 'dog', 'fan', 'water', 'printer',
];

async function roomCall(method, path, body) {
  const res = await fetch(path, { method, headers, body: body == null ? undefined : JSON.stringify(body) });
  const answer = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = new Error(answer.error ?? res.statusText);
    err.status = res.status;
    throw err;
  }
  return answer;
}

/** Declares a room (`previous` null) or changes one, as a person (403:
 *  the caller asks for the code first, see `asHuman`). The layout the
 *  server answers is taken at once. */
export async function saveRoom(room, previous = null) {
  const body = { name: room.name, icon: room.icon ?? '', aliases: room.aliases ?? [] };
  const layout = previous
    ? await roomCall('PUT', `/api/home/rooms/${encodeURIComponent(previous)}`, body)
    : await roomCall('POST', '/api/home/rooms', body);
  home.config = layout;
  return layout;
}

// ---- the menu's pages ----------------------------------------------------------

/** What the menu's pages are decided on (`pages.js`): the devices shown,
 *  the layout, the plan, the meters; `extra`: `people`, `voice`. */
export function pageFacts(extra = {}) {
  return {
    devices: Object.values(hub.devices).filter((d) => !hidden(d.id)),
    kindOf: deviceKind,
    config: home.config ?? {},
    plan: home.plan,
    energy: home.energy,
    ...extra,
  };
}

/** A person's choices (`home.json` `pages`): `{ energie: 'never', … }`. */
export const pageChoices = () => home.config?.pages ?? {};

/** The cameras' page is the doorbell's when only Ring tells its events. */
export const camerasAsDoorbell = () => camerasOnlyRing(Object.values(hub.devices).filter((d) => !hidden(d.id)), home.config);

/** Shows or hides pages (`{ energie: 'never', infra: 'auto' }`), as a
 *  person (403: the caller asks for the code first, see `asHuman`). */
export async function savePages(choices) {
  home.config = await roomCall('PUT', '/api/home/pages', choices);
  return home.config;
}

/** Forgets a declared room (its devices keep their labels). */
export async function removeRoom(name) {
  home.config = await roomCall('DELETE', `/api/home/rooms/${encodeURIComponent(name)}`);
}

// ---- ambiances -----------------------------------------------------------------

export const looks = $state({ list: null, busy: null });

/** The ambiances Moli knows (`GET /api/ambiances`), fetched once. */
export async function loadAmbiances() {
  if (looks.list) return;
  try {
    const res = await fetch('/api/ambiances');
    if (res.ok) looks.list = await res.json();
  } catch {
    /* next time */
  }
}

/** Several lights at once: `{ ambiance }`, `{ color }` or `{ white }`.
 *  A protected room asks for the code, then tries again as a human. */
export async function setLook(lights, look, label) {
  const key = JSON.stringify(look);
  looks.busy = key;
  try {
    const res = await fetch('/api/ambiance', { method: 'POST', headers, body: JSON.stringify({ lights, ...look }) });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.error ?? res.statusText);
    if (body.held?.length) {
      home.held = { label, reason: body.held[0].reason, custom: () => setLook(lights, look, label) };
      return 'held';
    }
    if (body.failed?.length) {
      const n = body.failed.length;
      note(t('commun.lampes_pas_suivi', { label, count: n, error: humanError(body.failed[0].error) }), 'error');
    } else if (!body.done?.length) {
      note(t('commun.aucune_lampe', { label }), 'info');
    }
    return 'ok';
  } catch (err) {
    note(t('commun.echec', { label, error: humanError(err.message) }), 'error');
    return 'error';
  } finally {
    if (looks.busy === key) looks.busy = null;
  }
}

function humanError(message) {
  if (/unreachable|injoignable|did not answer|no answer/i.test(message)) return t('commun.appareil_injoignable');
  if (/read-only|lecture seule|cannot be set/i.test(message)) return t('commun.non_pilotable');
  return message;
}

export function toggle(id, label, point = null) {
  const d = device(id);
  const key = point ?? powerKey(d);
  if (!key) return;
  const current = value(id, key);
  const next = typeof current === 'string' ? (truthy(current) ? 'OFF' : 'ON') : !truthy(current);
  return act(`${id}/${key}`, next, label ?? nameOf(id));
}

/** « C'est moi » : opens a human session with the code, then lets the
 *  held order through. Returns an error message or null. */
export async function confirmHeld(pin) {
  const held = home.held;
  if (!held) return null;
  const error = await login(pin);
  if (error) return /verrou|locked/i.test(error) ? t('commun.trop_d_essais') : t('commun.code_incorrect');
  if (held.custom) {
    home.held = null;
    await held.custom();
    return null;
  }
  await decide({ id: held.id, point: held.point }, true);
  held.onApproved?.();
  home.held = null;
  return null;
}

export async function dismissHeld() {
  const held = home.held;
  home.held = null;
  if (held?.custom) held.onCancel?.();
  else if (held) await decide({ id: held.id, point: held.point }, false);
}

export { refreshSession };

// ---- formatting ----------------------------------------------------------------

export function num(v, digits = 0) {
  if (v == null || !Number.isFinite(Number(v))) return '—';
  return Number(v).toLocaleString(locale(), { maximumFractionDigits: digits, minimumFractionDigits: digits });
}

/** A temperature: « 18 » when whole, « 19,5 » otherwise. */
export function deg(v) {
  return Number.isInteger(Number(v)) ? num(v, 0) : num(v, 1);
}

export function clock(ts) {
  return new Date(ts).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
}

/** « aujourd'hui à 07:42 », « hier à 22:24 », « le 12 oct. à 08:10 ». */
export function dayAndTime(ts, now = Date.now()) {
  const day = (x) => new Date(x).toDateString();
  const time = clock(ts);
  if (day(ts) === day(now)) return t('commun.quand.aujourdhui', { time });
  if (day(ts) === day(now - 86_400_000)) return t('commun.quand.hier', { time });
  const date = new Date(ts).toLocaleDateString(locale(), { day: 'numeric', month: 'short' });
  return t('commun.quand.le', { date, time });
}

/** « Samedi 3 octobre »: only the first letter up. */
export function longDate(ts) {
  const s = new Date(ts).toLocaleDateString(locale(), { weekday: 'long', day: 'numeric', month: 'long' });
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** When a camera last saw someone (or someone rang), from the history:
 *  the reported value refreshes all the time, the event does not. */
export async function lastSeen(id, keys = ['person', 'doorbell'], hours = 48) {
  const times = await Promise.all(
    keys.map((k) =>
      fetch(`/api/history?point=${encodeURIComponent(`${id}/${k}`)}&hours=${hours}&points=2000`)
        .then((r) => (r.ok ? r.json() : null))
        .then((s) => {
          const on = (v) => v === true || (typeof v === 'number' && v > 0);
          const hits = (s?.raw ?? []).filter(([, v]) => on(v));
          return hits.length ? hits.at(-1)[0] : null;
        })
        .catch(() => null),
    ),
  );
  const found = times.filter(Boolean);
  return found.length ? Math.max(...found) : null;
}

export function greeting(ts) {
  const h = new Date(ts).getHours();
  if (h < 5) return t('commun.salut.nuit');
  if (h < 12) return t('commun.salut.matin');
  if (h < 18) return t('commun.salut.apres_midi');
  return t('commun.salut.soir');
}

export function relative(ts, now = Date.now()) {
  if (!ts) return '';
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 60) return t('commun.relatif.instant');
  if (s < 3600) return t('commun.relatif.minutes', { n: Math.round(s / 60) });
  if (s < 86400) return t('commun.relatif.heures', { n: Math.round(s / 3600) });
  return t('commun.relatif.jours', { n: Math.round(s / 86400) });
}
