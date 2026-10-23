// What each card of the home page is: its name, its icon, its settings
// (with their choices, read from the house) and what it is about. The
// pure arrangement logic is `../lib/layout.js`; the components are mounted
// by `Card.svelte`.

import { hub, home, device, nameOf, roomOf, roomList, deviceKind, hidden, heatedRoom } from '../lib/home.svelte.js';
import { isPrinter } from '../lib/printers.js';
import { t } from '../../lib/i18n.svelte.js';

const shown = () => Object.values(hub.devices).filter((d) => !hidden(d.id));
const choice = (d) => ({ value: d.id, label: nameOf(d.id) });
const ofKind = (kind) => shown().filter((d) => deviceKind(d) === kind);

/** The light tiles the page may show: the favourites first, then one per
 *  room that has lights no favourite speaks for (its Hue room, else its
 *  lamps together), so that every light of the house has a tile. */
export function lightTiles() {
  const favourites = (home.config?.favorites?.lights ?? []).map((l) => ({ ...l, also: l.also ?? [] }));
  const covered = new Set(favourites.flatMap((l) => [l.id, ...l.also]));
  const rooms = roomList()
    .filter((r) => r.group || r.lights.length)
    .filter((r) => ![r.group?.id, ...r.lights.map((d) => d.id)].some((id) => id && covered.has(id)))
    .map((r) => {
      const first = r.group ?? r.lights[0];
      return { id: first.id, name: r.name, icon: r.icon, also: r.lights.map((d) => d.id).filter((id) => id !== first.id) };
    });
  return [...favourites, ...rooms];
}

/** The clims of the page: the favourites that exist (a Netatmo room is the
 *  heating card's), then any other air conditioner. */
export function climates() {
  const favourites = (home.config?.favorites?.climate ?? []).filter((c) => hub.devices[c.id] && !heatedRoom(hub.devices[c.id]));
  const rest = ofKind('climate')
    .filter((d) => !heatedRoom(d) && !favourites.some((c) => c.id === d.id))
    .map((d) => ({ id: d.id, name: nameOf(d.id) }));
  return [...favourites, ...rest];
}

/** The rooms the heating card may show, as the layout orders them. */
export function heatedRooms() {
  const order = (home.config?.rooms ?? []).map((r) => r.name);
  return shown()
    .filter(heatedRoom)
    .map((d) => roomOf(d) ?? nameOf(d.id))
    .sort((a, b) => {
      const i = (n) => (order.includes(n) ? order.indexOf(n) : order.length);
      return i(a) - i(b) || a.localeCompare(b);
    });
}

export const cameras = () => shown().filter((d) => d.camera);

const OUTSIDE_FACTS = ['air', 'uv', 'humidity', 'pool'];

/**
 * Each card: `label` (a catalogue key), `icon`, `options()` (the defaults),
 * `fields` (its settings: `toggle`, `pick` one of `choices()`, `order` some
 * of `choices()` in that order; null means all of them), `about` (the
 * option that says which room or device it is).
 */
export const CARDS = {
  sky: { label: 'maison.cartes.sky', icon: 'sun', options: () => ({ map: true }), fields: [{ key: 'map', kind: 'toggle', label: 'maison.cartes.sky_carte' }] },
  modes: { label: 'maison.cartes.modes', icon: 'home', options: () => ({}), fields: [] },
  notices: { label: 'maison.cartes.notices', icon: 'alert', options: () => ({ allGood: true }), fields: [{ key: 'allGood', kind: 'toggle', label: 'maison.cartes.notices_tout_va_bien' }] },
  heating: {
    label: 'maison.cartes.heating',
    icon: 'radiator',
    options: () => ({ only: null, next: true, planning: true, away: true }),
    fields: [
      { key: 'only', kind: 'order', label: 'maison.cartes.heating_pieces', choices: () => heatedRooms().map((name) => ({ value: name, label: name })) },
      { key: 'next', kind: 'toggle', label: 'maison.cartes.heating_prochain' },
      { key: 'planning', kind: 'toggle', label: 'maison.cartes.heating_planning' },
      { key: 'away', kind: 'toggle', label: 'maison.cartes.heating_absence' },
    ],
  },
  lights: {
    label: 'maison.cartes.lights',
    icon: 'light',
    options: () => ({ tiles: null, ambiance: true }),
    fields: [
      { key: 'tiles', kind: 'order', label: 'maison.cartes.lights_tuiles', choices: () => lightTiles().map((l) => ({ value: l.id, label: l.name })) },
      { key: 'ambiance', kind: 'toggle', label: 'maison.cartes.lights_ambiance' },
    ],
  },
  climate: {
    label: 'maison.cartes.climate',
    icon: 'air-conditioner',
    options: () => ({ devices: null }),
    fields: [{ key: 'devices', kind: 'order', label: 'maison.cartes.climate_appareils', choices: () => climates().map((c) => ({ value: c.id, label: c.name })) }],
  },
  remote: {
    label: 'maison.cartes.remote',
    icon: 'remote',
    about: 'device',
    options: () => ({ device: null }),
    fields: [{ key: 'device', kind: 'pick', label: 'maison.cartes.appareil', none: 'maison.cartes.remote_salon', choices: () => ofKind('tv').map(choice) }],
  },
  outside: {
    label: 'maison.cartes.outside',
    icon: 'flower-outline',
    options: () => ({ facts: null }),
    fields: [{ key: 'facts', kind: 'order', label: 'maison.cartes.outside_faits', choices: () => OUTSIDE_FACTS.map((f) => ({ value: f, label: t(`maison.cartes.outside_${f}`) })) }],
  },
  doorbell: {
    label: 'maison.cartes.doorbell',
    icon: 'bell',
    options: () => ({ camera: null, modes: true }),
    fields: [
      { key: 'camera', kind: 'pick', label: 'maison.cartes.doorbell_camera', none: 'maison.cartes.doorbell_auto', choices: () => cameras().map(choice) },
      { key: 'modes', kind: 'toggle', label: 'maison.cartes.doorbell_modes' },
    ],
  },
  music: {
    label: 'maison.cartes.music',
    icon: 'speaker',
    options: () => ({ device: null }),
    fields: [{ key: 'device', kind: 'pick', label: 'maison.cartes.music_enceinte', none: 'maison.cartes.music_celle_qui_joue', choices: () => ofKind('speaker').map(choice) }],
  },
  today: { label: 'maison.cartes.today', icon: 'clock', options: () => ({}), fields: [] },
  energy: { label: 'maison.cartes.energy', icon: 'bolt', options: () => ({}), fields: [] },
  routines: { label: 'maison.cartes.routines', icon: 'hand', options: () => ({}), fields: [] },
  room: {
    label: 'maison.cartes.room',
    icon: 'light-group',
    about: 'room',
    options: () => ({ room: null }),
    fields: [{ key: 'room', kind: 'pick', label: 'maison.cartes.piece', choices: () => roomList().map((r) => ({ value: r.name, label: r.name })) }],
  },
  speaker: {
    label: 'maison.cartes.speaker',
    icon: 'speaker',
    about: 'device',
    options: () => ({ device: null }),
    fields: [{ key: 'device', kind: 'pick', label: 'maison.cartes.appareil', choices: () => ofKind('speaker').map(choice) }],
  },
  diffuser: {
    label: 'maison.cartes.diffuser',
    icon: 'fan',
    about: 'device',
    options: () => ({ device: null }),
    fields: [{ key: 'device', kind: 'pick', label: 'maison.cartes.appareil', choices: () => ofKind('diffuser').map(choice) }],
  },
  camera: {
    label: 'maison.cartes.camera',
    icon: 'cctv',
    about: 'device',
    options: () => ({ device: null, refresh: 15 }),
    fields: [
      { key: 'device', kind: 'pick', label: 'maison.cartes.appareil', choices: () => cameras().map(choice) },
      { key: 'refresh', kind: 'pick', label: 'maison.cartes.camera_rafraichir', choices: () => [5, 15, 60].map((s) => ({ value: s, label: t('maison.cartes.camera_secondes', { n: s }) })) },
    ],
  },
  presence: { label: 'maison.cartes.presence', icon: 'people', options: () => ({}), fields: [] },
  assistant: { label: 'maison.cartes.assistant', icon: 'robot', options: () => ({}), fields: [] },
  machine: { label: 'maison.cartes.machine', icon: 'server', options: () => ({}), fields: [] },
  printer: {
    label: 'maison.cartes.printer',
    icon: 'printer3d',
    about: 'device',
    options: () => ({ device: null }),
    fields: [{ key: 'device', kind: 'pick', label: 'maison.cartes.appareil', choices: () => shown().filter(isPrinter).map(choice) }],
  },
};

/** A card's options with the defaults filled in. */
export const optionsOf = (card) => ({ ...(CARDS[card.type]?.options?.() ?? {}), ...(card.options ?? {}) });

/** What a card is called: the person's title, else Moli's name for it,
 *  with the room or device it is about. */
export function cardLabel(card) {
  if (card.title) return card.title;
  const def = CARDS[card.type];
  if (!def) return card.type;
  const o = optionsOf(card);
  if (def.about === 'room' && o.room) return o.room;
  if (def.about === 'device' && o.device) return `${t(def.label)} · ${nameOf(o.device)}`;
  return t(def.label);
}

/** A word on the card under its name, in the editor. */
export function cardSummary(card) {
  const def = CARDS[card.type];
  if (!def) return t('maison.modifier.carte_inconnue');
  const o = optionsOf(card);
  if (def.about === 'room') return o.room && roomList().some((r) => r.name === o.room) ? t('maison.cartes.room') : t('maison.modifier.introuvable');
  if (def.about === 'device') return o.device && device(o.device) ? nameOf(o.device) : t('maison.modifier.introuvable');
  if (card.type === 'heating') return t('maison.cartes.heating_resume', { count: (o.only ?? heatedRooms()).length });
  if (card.type === 'lights') return t('maison.cartes.lights_resume', { count: (o.tiles ?? lightTiles().map((l) => l.id)).length });
  return t(`maison.cartes.${card.type}_resume`);
}

/** What the house offers the catalogue: its rooms, its devices by kind. */
export function houseFacts() {
  const kindOf = (d) => (isPrinter(d) ? 'printer' : deviceKind(d));
  return {
    rooms: roomList().map((r) => ({ name: r.name })),
    devices: shown()
      .map((d) => ({ id: d.id, name: nameOf(d.id), kind: kindOf(d) }))
      .filter((d) => ['speaker', 'diffuser', 'camera', 'printer'].includes(d.kind)),
  };
}
