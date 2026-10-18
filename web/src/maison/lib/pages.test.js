import { test } from 'node:test';
import assert from 'node:assert/strict';
import { justified, shown, visiblePages, PAGE_IDS } from './pages.js';

const point = (key, extra = {}) => ({ key, access: { read: true, write: false }, ...extra });

// A made-up house: lights, a speaker, a mist diffuser, radiators, a
// camera, the weather; no TV, no meter, no printer, no plan.
const house = [
  { id: 'hue:1', points: [point('on', { access: { write: true }, semantic: 'on_off' })] },
  { id: 'tuya:diff', points: [point('mist_mode', { access: { write: true } })] },
  { id: 'alexa:1', points: [point('playing', { access: { write: true } }), point('volume'), point('say', { access: { write: true }, kind: { type: 'text' } })] },
  { id: 'z2m:radiator', points: [point('temperature'), point('target_temperature', { access: { write: true } }), point('heating')] },
  { id: 'reolink:door', camera: true, points: [point('motion')] },
  { id: 'meteo:home', points: [point('temperature'), point('weather_code'), point('daylight')] },
];
const kindOf = (d) => (d.camera ? 'camera' : d.points.some((p) => p.key === 'key' && p.access?.write) ? 'tv' : 'sensor');
const facts = { devices: house, kindOf, config: { outdoor: { weather: 'meteo:home' } }, plan: { floors: [] }, energy: { available: false }, people: { zones: [], phones: [] }, voice: false };

test('this house keeps what it has and loses the rest', () => {
  const v = visiblePages(facts, {});
  const kept = PAGE_IDS.filter((id) => v[id]);
  assert.deepEqual(kept, ['pieces', 'cameras', 'dehors', 'automatismes', 'moli']);
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

test('a camera or a favourite opens the cameras’ page', () => {
  const noCamera = house.filter((d) => !d.camera);
  assert.equal(justified({ ...facts, devices: noCamera }).cameras, false);
  assert.equal(justified({ ...facts, devices: noCamera, config: { favorites: { cameras: [{ id: 'x' }] } } }).cameras, true);
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
