import { test } from 'node:test';
import assert from 'node:assert/strict';
import { justified, shown, visiblePages, camerasOnlyRing, PAGE_IDS } from './pages.js';
import { timedOn, nextHeating, weekday } from './today.js';

const point = (key, extra = {}) => ({ key, access: { read: true, write: false }, ...extra });

// A house like the one the redesign was made with: Hue, a Tuya diffuser,
// Echos, Netatmo heating, a Ring doorbell and two Ring cameras, the weather.
const house = [
  { id: 'hue:1', points: [point('on', { access: { write: true }, semantic: 'on_off' })] },
  { id: 'tuya:diff', points: [point('mist_mode', { access: { write: true } })] },
  { id: 'alexa:1', points: [point('playing', { access: { write: true } }), point('volume'), point('say', { access: { write: true }, kind: { type: 'text' } })] },
  { id: 'netatmo:home', service: true, manufacturer: 'Netatmo', points: [point('mode'), point('schedule')] },
  { id: 'netatmo:room', manufacturer: 'Netatmo', points: [point('temperature'), point('setpoint'), point('setpoint_mode'), point('heating')] },
  { id: 'ring:bell', manufacturer: 'Ring', points: [point('doorbell'), point('motion'), point('battery')] },
  { id: 'ring:cam', manufacturer: 'Ring', points: [point('motion'), point('battery')] },
  { id: 'meteo:home', points: [point('temperature'), point('weather_code'), point('daylight')] },
];
const kindOf = (d) => (d.camera ? 'camera' : d.points.some((p) => p.key === 'key' && p.access?.write) ? 'tv' : 'sensor');
const facts = { devices: house, kindOf, config: { outdoor: { weather: 'meteo:home' } }, plan: { floors: [] }, energy: { available: false }, people: { zones: [], phones: [] }, voice: false };

test('this house keeps what it has and loses the rest', () => {
  const v = visiblePages(facts, {});
  const kept = PAGE_IDS.filter((id) => v[id]);
  assert.deepEqual(kept, ['pieces', 'chauffage', 'cameras', 'dehors', 'automatismes', 'moli']);
});

test('each page comes back by itself with its device', () => {
  const tv = { id: 'philips:tv', points: [point('key', { access: { write: true } })] };
  assert.equal(justified({ ...facts, devices: [...house, tv] }).salon, true);
  assert.equal(justified({ ...facts, config: { salon: { tv: 'x' } } }).salon, true);
  assert.equal(justified({ ...facts, energy: { available: true } }).energie, true);
  assert.equal(justified({ ...facts, config: { energy: {} } }).energie, true);
  const panel = { id: 'x:pv', points: [point('pv_power', { semantic: 'power' })] };
  assert.equal(justified(facts).solaire, false);
  assert.equal(justified({ ...facts, devices: [...house, panel] }).solaire, true);
  const printer = { id: 'k:1', points: [point('progress'), point('nozzle_temperature')] };
  assert.equal(justified({ ...facts, devices: [...house, printer] }).impression, true);
  assert.equal(justified({ ...facts, devices: [...house, { id: 'ipp:1', printer: true, points: [] }] }).impression, true);
  assert.equal(justified({ ...facts, plan: { floors: [{ id: 'rdc' }] } }).plan, true);
  assert.equal(justified({ ...facts, people: { zones: [{ id: 'z' }], phones: [] } }).carte, true);
  assert.equal(justified({ ...facts, people: { zones: [], phones: [{ id: 'p' }] } }).carte, true);
  const sat = { id: 'esphome:sat', points: [point('state', { kind: { type: 'enum', values: ['idle', 'listening', 'thinking', 'speaking'] } })] };
  assert.equal(justified({ ...facts, devices: [...house, sat] }).parler, true);
  assert.equal(justified({ ...facts, voice: true }).parler, true);
  assert.equal(justified(facts).parler, false, 'an Echo that says a sentence is no satellite');
});

test('an empty house keeps only what always serves', () => {
  const v = justified({});
  assert.deepEqual(
    PAGE_IDS.filter((id) => v[id]),
    ['pieces', 'automatismes', 'moli'],
  );
  assert.equal(v.infra, false, 'the machines are the Atelier’s');
});

test('a person forces a page in or out', () => {
  const auto = justified(facts);
  assert.equal(shown('energie', auto, { energie: 'always' }), true);
  assert.equal(shown('dehors', auto, { dehors: 'never' }), false);
  assert.equal(shown('dehors', auto, { dehors: 'auto' }), true);
  assert.equal(shown('infra', auto, { infra: 'always' }), true);
  assert.equal(shown('infra', auto, null), false);
});

test('the cameras are a doorbell when only Ring tells its events', () => {
  assert.equal(camerasOnlyRing(house, {}), true);
  assert.equal(camerasOnlyRing([...house, { id: 'reolink:1', camera: true, points: [] }], {}), false);
  assert.equal(camerasOnlyRing(house, { favorites: { cameras: [{ id: 'reolink:1' }] } }), false);
  assert.equal(camerasOnlyRing([], {}), false);
});

// ---- the day's timed automations -----------------------------------------------------

const auto = (id, name, at, { days = [], point = 'netatmo:room/setpoint', approved = true, enabled = true } = {}) => ({
  approved,
  automation: {
    id,
    name,
    enabled,
    graph: {
      nodes: [
        { id: 't', type: 'at_time', at, days },
        { id: 's', type: 'set', point, value: 18 },
      ],
      edges: [{ from: 't', to: 's' }],
    },
  },
});

const heating = (p) => p.startsWith('netatmo:');
// Sunday 18 October 2026, 07:42.
const now = new Date(2026, 9, 18, 7, 42);

test('the day is counted Monday first', () => {
  assert.equal(weekday(now), 7);
  assert.equal(weekday(new Date(2026, 9, 19)), 1);
});

test('today: past, to come, only the live ones', () => {
  const list = [
    auto('a', 'Chauffage : journée', '05:15'),
    auto('b', 'Chauffage : nuit', '21:00'),
    auto('c', 'Chauffage : relance', '13:15'),
    auto('d', 'Brouillon', '09:00', { approved: false }),
    auto('e', 'Éteint', '10:00', { enabled: false }),
    auto('f', 'En semaine', '08:00', { days: [1, 2, 3, 4, 5] }),
    auto('h', 'Dimanche', '11:00', { days: [7], point: 'hue:1/on' }),
    auto('g', 'Volets', '08:30', { point: 'tuya:1/control' }),
  ];
  const today = timedOn(list, now, heating);
  assert.deepEqual(
    today.map((x) => [x.name, x.done, x.heating]),
    [
      ['Chauffage : journée', true, true],
      ['Volets', false, false],
      ['Dimanche', false, false],
      ['Chauffage : relance', false, true],
      ['Chauffage : nuit', false, true],
    ],
  );
  assert.deepEqual(nextHeating(list, now, heating), { name: 'Chauffage : relance', minutes: 13 * 60 + 15, tomorrow: false });
});

test('after the last change of the day, tomorrow’s first', () => {
  const list = [auto('a', 'Journée', '05:15'), auto('b', 'Nuit', '21:00'), auto('c', 'Mardi', '04:00', { days: [2] })];
  const late = new Date(2026, 9, 18, 22, 0);
  assert.deepEqual(nextHeating(list, late, heating), { name: 'Journée', minutes: 5 * 60 + 15, tomorrow: true });
  const monday = [...list, auto('l', 'Lundi', '04:00', { days: [1] })];
  assert.deepEqual(nextHeating(monday, late, heating), { name: 'Lundi', minutes: 4 * 60, tomorrow: true }, 'Monday comes after Sunday');
  assert.equal(nextHeating([], late, heating), null);
  assert.equal(nextHeating([auto('v', 'Volets', '23:00', { point: 'tuya:1/control' })], late, heating), null);
});
