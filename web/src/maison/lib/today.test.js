import { test } from 'node:test';
import assert from 'node:assert/strict';
import { timedOn, weekday } from './today.js';

const auto = (id, name, at, { days = [], point = 'z2m:radiator/target_temperature', approved = true, enabled = true } = {}) => ({
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
  assert.deepEqual(
    timedOn(list, now).map((x) => [x.name, x.done]),
    [
      ['Chauffage : journée', true],
      ['Volets', false],
      ['Dimanche', false],
      ['Chauffage : relance', false],
      ['Chauffage : nuit', false],
    ],
  );
});

test('tomorrow: its own days, nothing done yet', () => {
  const list = [auto('a', 'Journée', '05:15'), auto('c', 'Mardi', '04:00', { days: [2] }), auto('l', 'Lundi', '04:00', { days: [1] })];
  assert.deepEqual(
    timedOn(list, now, 1).map((x) => [x.name, x.done]),
    [
      ['Lundi', false],
      ['Journée', false],
    ],
    'Monday comes after Sunday',
  );
  assert.deepEqual(timedOn([], now), []);
});
