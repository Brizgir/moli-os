// How the home page is arranged: an ordered list of cards per screen (the
// big screen, the phone), each with a width and its settings. The page lays
// the list out itself (a grid that packs densely): nothing overlaps, nothing
// leaves a hole, and a finger only aims at a rank, never a pixel. Pure and
// tested (`layout.test.js`); the state and the server calls are in
// `dashboard.svelte.js`, the cards themselves in `../accueil/cards.js`.

/** What a card can be (the server accepts these only: `CARD_TYPES` of
 *  `crates/moli-api/src/dashboard.rs`). */
export const CARD_TYPES = ['sky', 'modes', 'notices', 'heating', 'lights', 'climate', 'remote', 'outside', 'doorbell', 'music', 'today', 'energy', 'routines', 'room', 'speaker', 'diffuser', 'camera', 'presence', 'assistant', 'machine', 'printer'];

/** A card's width on the big screen: a third, two thirds, the whole row.
 *  On the phone (one column) it says how much the card tells. */
export const SIZES = ['s', 'm', 'l'];
export const SPAN = { s: 1, m: 2, l: 3 };
export const SCREENS = ['desktop', 'phone'];
export const MAX_CARDS = 60;
export const MAX_LAYOUTS = 12;

/** The phone's arrangement below this width (the tab bar's breakpoint). */
export const screenFor = (width) => (width < 768 ? 'phone' : 'desktop');

/** The cards every house may have once, in the order the page had them
 *  before any arrangement existed: the main column (two thirds) and the
 *  side column (a third), interleaved so a dense grid draws them alike. A
 *  card without its data (no TV, no meter…) stays silent by itself. */
const DEFAULT_DESKTOP = [
  ['sky', 'l'],
  ['modes', 'l'],
  ['notices', 'l'],
  ['heating', 'm'],
  ['doorbell', 's'],
  ['lights', 'm'],
  ['music', 's'],
  ['climate', 'm'],
  ['today', 's'],
  ['remote', 's'],
  ['outside', 's'],
  ['energy', 's'],
  ['routines', 's'],
];

/** The phone: one column, the main cards first, as the page stacked them. */
const DEFAULT_PHONE = ['sky', 'modes', 'notices', 'heating', 'lights', 'climate', 'remote', 'outside', 'doorbell', 'music', 'today', 'energy', 'routines'];

/** The arrangement a house shows without any stored: today's page. */
export function defaultLayout(screen) {
  const cards = screen === 'phone' ? DEFAULT_PHONE.map((type) => [type, 'l']) : DEFAULT_DESKTOP;
  return { cards: cards.map(([type, size]) => ({ id: `${type}0`, type, size })) };
}

/** A whole set of layouts as the server keeps it: `default` and the named ones. */
export const defaultLayouts = () => ({ default: { desktop: defaultLayout('desktop'), phone: defaultLayout('phone') } });

/**
 * What the page shows: the person's own layouts, else the house's, else
 * the default; within them the named layout `active` (else `default`), on
 * `screen`. `stored`: whether anything was kept for this viewer.
 */
export function resolve({ house = null, mine = null } = {}, screen, active = 'default') {
  const set = mine ?? house ?? null;
  const key = set && active in set ? active : 'default';
  const cards = set?.[key]?.[screen]?.cards;
  return {
    cards: Array.isArray(cards) ? cards : defaultLayout(screen).cards,
    stored: !!set,
    scope: mine ? 'mine' : house ? 'house' : null,
    active: key,
  };
}

/** The named layouts of a set, `default` first. */
export function names(set) {
  const keys = Object.keys(set ?? {}).filter((k) => k !== 'default');
  return [{ key: 'default', name: null }, ...keys.map((key) => ({ key, name: set[key]?.name ?? key }))];
}

/** A key for a named layout, from what the person typed. */
export function slug(name) {
  const s = String(name ?? '')
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32);
  return s === 'default' ? 'default-2' : s;
}

/** A fresh card id (4 to 16 lower-case letters or digits), unlike `taken`. */
export function newId(taken = []) {
  const alphabet = 'abcdefghijklmnopqrstuvwxyz0123456789';
  for (;;) {
    let id = '';
    for (let i = 0; i < 8; i++) id += alphabet[Math.floor(Math.random() * alphabet.length)];
    if (!taken.includes(id)) return id;
  }
}

// ---- editing a list of cards (each returns a new list) ---------------------------

const ids = (cards) => cards.map((c) => c.id);

/** The card moved from rank `from` to rank `to`. */
export function move(cards, from, to) {
  if (from === to || from < 0 || from >= cards.length) return cards;
  const out = cards.slice();
  const [card] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(to, out.length)), 0, card);
  return out;
}

/** The card `id` one step up (`-1`) or down (`+1`) among the shown ones
 *  (a hidden card keeps its place). */
export function shift(cards, id, delta) {
  const shown = cards.filter((c) => !c.hidden);
  const i = shown.findIndex((c) => c.id === id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= shown.length) return cards;
  return move(cards, cards.indexOf(shown[i]), cards.indexOf(shown[j]));
}

/** The card `id` with `patch` applied (`{ size }`, `{ hidden }`, `{ title }`, `{ options }`). */
export function update(cards, id, patch) {
  return cards.map((c) => {
    if (c.id !== id) return c;
    const next = { ...c, ...patch };
    if (patch.options) next.options = { ...(c.options ?? {}), ...patch.options };
    return next;
  });
}

/** A card hidden (kept with its settings) or shown again, at the end. */
export function setHidden(cards, id, hidden) {
  const card = cards.find((c) => c.id === id);
  if (!card) return cards;
  const rest = cards.filter((c) => c.id !== id);
  return hidden ? [...rest, { ...card, hidden: true }] : [...rest, { ...card, hidden: false }];
}

/** A new card at the end, or at `at`. */
export function add(cards, card, at = cards.length) {
  if (cards.length >= MAX_CARDS) return cards;
  const out = cards.slice();
  out.splice(at, 0, { size: 'm', ...card, id: card.id ?? newId(ids(cards)) });
  return out;
}

export const remove = (cards, id) => cards.filter((c) => c.id !== id);

/** A copy of the card `id`, right after it. */
export function duplicate(cards, id) {
  const i = cards.findIndex((c) => c.id === id);
  if (i < 0) return cards;
  const copy = { ...cards[i], id: newId(ids(cards)), hidden: false };
  if (copy.options) copy.options = JSON.parse(JSON.stringify(copy.options));
  return add(cards, copy, i + 1);
}

// ---- editing a set of layouts -----------------------------------------------------

/** The set with `cards` in place for `key` on `screen` (a missing layout
 *  starts from the default). */
export function withCards(set, key, screen, cards) {
  const base = set?.[key] ?? { ...(key === 'default' ? {} : { name: key }), ...defaultLayouts().default };
  return { ...(set ?? defaultLayouts()), [key]: { ...base, [screen]: { cards } } };
}

/** A named layout added, copied from `from` (else the default). */
export function addLayout(set, name, from = 'default') {
  const all = set ?? defaultLayouts();
  const key = slug(name);
  if (!key || key in all || Object.keys(all).length >= MAX_LAYOUTS) return { set: all, key: null };
  const source = all[from] ?? all.default;
  return { set: { ...all, [key]: { name: String(name).trim(), desktop: source.desktop, phone: source.phone } }, key };
}

export function removeLayout(set, key) {
  if (key === 'default' || !set) return set;
  const { [key]: _gone, ...rest } = set;
  return rest;
}

// ---- the catalogue ----------------------------------------------------------------

/** The cards a house may add once each (the rest are per room or device). */
export const HOUSE_CARDS = ['sky', 'modes', 'notices', 'heating', 'lights', 'climate', 'remote', 'outside', 'doorbell', 'music', 'today', 'energy', 'routines', 'presence', 'assistant', 'machine'];

/** What a card is about, for telling two of the same type apart. */
export function keyOf(card) {
  const o = card.options ?? {};
  return `${card.type}:${o.room ?? o.device ?? ''}`;
}

/**
 * What the house can offer, against the cards already there:
 * `{ type, options, label, present }` with `present` `shown`, `hidden` or
 * null. `facts`: `rooms` (`[{ name }]`), `devices` (`[{ id, name, kind }]`).
 */
export function catalogue({ rooms = [], devices = [] } = {}, cards = []) {
  const present = (type, options = {}) => {
    const key = keyOf({ type, options });
    const found = cards.find((c) => keyOf(c) === key);
    return found ? (found.hidden ? 'hidden' : 'shown') : null;
  };
  const byKind = (kind, type) =>
    devices
      .filter((d) => d.kind === kind)
      .map((d) => ({ type, options: { device: d.id }, label: d.name, present: present(type, { device: d.id }) }));
  return {
    rooms: rooms.map((r) => ({ type: 'room', options: { room: r.name }, label: r.name, present: present('room', { room: r.name }) })),
    devices: [...byKind('speaker', 'speaker'), ...byKind('diffuser', 'diffuser'), ...byKind('camera', 'camera'), ...byKind('printer', 'printer')],
    house: HOUSE_CARDS.map((type) => ({ type, options: {}, label: null, present: present(type) })),
  };
}
