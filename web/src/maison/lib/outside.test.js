import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sunFacts, hoursMinutes, groundFloor, outdoorSpaces, miniPlan, placedIndoors, temperatureDay, sparkPath } from './outside.js';

const ms = (iso) => Date.parse(iso);

test('the sun: before, during and after the day', () => {
  const rise = '2026-10-19T08:12';
  const set = '2026-10-19T18:48';
  const day = sunFacts(rise, set, ms('2026-10-19T13:30'));
  assert.equal(day.phase, 'day');
  assert.ok(day.pos > 0.49 && day.pos < 0.51, `${day.pos}`);
  assert.equal(day.length, ms(set) - ms(rise));
  assert.equal(day.until, ms(set) - ms('2026-10-19T13:30'));
  assert.equal(day.noon, ms('2026-10-19T13:30'));
  const dawn = sunFacts(rise, set, ms('2026-10-19T06:00'));
  assert.equal(dawn.phase, 'before');
  assert.equal(dawn.pos, 0);
  assert.equal(dawn.until, ms(rise) - ms('2026-10-19T06:00'));
  const night = sunFacts(rise, set, ms('2026-10-19T22:00'));
  assert.equal(night.phase, 'after');
  assert.equal(night.pos, 1);
  assert.equal(night.until, ms('2026-10-20T08:12') - ms('2026-10-19T22:00'));
  assert.equal(sunFacts(null, set, 0), null);
  assert.equal(sunFacts(set, rise, 0), null);
});

test('a duration in hours and minutes', () => {
  assert.deepEqual(hoursMinutes(2 * 3_600_000 + 10 * 60_000), { h: 2, m: 10 });
  assert.deepEqual(hoursMinutes(45 * 60_000), { h: 0, m: 45 });
  assert.deepEqual(hoursMinutes(-5), { h: 0, m: 0 });
});

// A plot 6 m wide and 30 m long: the drive in front, the house, the
// terrace and the garden behind; the office upstairs.
const rect = (x, y, w, h) => [
  [x, y],
  [x + w, y],
  [x + w, y + h],
  [x, y + h],
];
const plan = {
  floors: [
    {
      id: 'r1',
      name: '1er',
      size: [600, 1000],
      elevation: 250,
      rooms: [{ id: 'bureau', name: 'Bureau', kind: 'office', poly: rect(0, 1000, 600, 1000) }],
      devices: [{ id: 'cam:bureau', x: 300, y: 1500 }],
    },
    {
      id: 'rdc',
      name: 'RDC',
      size: [600, 3000],
      elevation: 0,
      rooms: [
        { id: 'allee', name: 'Allée', kind: 'driveway', poly: rect(0, 0, 600, 800) },
        { id: 'salon', name: 'Salon', kind: 'living', poly: rect(0, 800, 600, 1000) },
        { id: 'terrasse', name: 'Terrasse', kind: 'terrace', poly: rect(0, 1800, 600, 300) },
        { id: 'jardin', name: 'Jardin', kind: 'garden', poly: rect(0, 2100, 600, 900) },
        { id: 'toit', name: 'Toit', kind: 'roof', poly: rect(0, 800, 600, 1000) },
      ],
      devices: [
        { id: 'cam:allee', x: 300, y: 400 },
        { id: 'cam:salon', x: 300, y: 1200 },
      ],
    },
  ],
};

test('the ground floor is the lowest drawn one, and its outdoor spaces largest first', () => {
  assert.equal(groundFloor(plan).id, 'rdc');
  assert.equal(groundFloor({ floors: [] }), null);
  assert.equal(groundFloor(null), null);
  const spaces = outdoorSpaces(groundFloor(plan));
  assert.deepEqual(
    spaces.map((s) => [s.id, s.area]),
    [
      ['jardin', 54],
      ['allee', 48],
      ['terrasse', 18],
    ],
  );
});

test('a long plot is turned to run across the card', () => {
  const pic = miniPlan(groundFloor(plan), 10);
  assert.equal(pic.turned, true);
  assert.equal(pic.w, 3000);
  assert.equal(pic.h, 600);
  assert.equal(pic.viewBox, '-10 -10 3020 620');
  // The roof is left out; the garden ends up on the right.
  assert.deepEqual(
    pic.rooms.map((r) => r.id),
    ['allee', 'salon', 'terrasse', 'jardin'],
  );
  const garden = pic.rooms.find((r) => r.id === 'jardin');
  assert.equal(garden.outdoor, true);
  assert.equal(garden.cx, 2550);
  assert.equal(garden.cy, 300);
  assert.deepEqual(pic.devices[0], { id: 'cam:allee', x: 400, y: 300 });
  assert.deepEqual(pic.house, { cx: 1300, cy: 300, w: 1000, h: 600 });
  // A square plot keeps its orientation.
  const square = miniPlan({ rooms: [{ id: 'a', name: 'A', kind: 'garden', poly: rect(100, 100, 500, 500) }] }, 0);
  assert.equal(square.turned, false);
  assert.deepEqual(square.rooms[0].poly[0], [0, 0]);
  assert.equal(miniPlan({ rooms: [] }), null);
});

test('a camera is indoors when the plan puts it in a room that is', () => {
  assert.equal(placedIndoors(plan, 'cam:salon'), true);
  assert.equal(placedIndoors(plan, 'cam:bureau'), true);
  assert.equal(placedIndoors(plan, 'cam:allee'), false);
  assert.equal(placedIndoors(plan, 'cam:nulle-part'), false);
  assert.equal(placedIndoors(null, 'cam:salon'), false);
});

test("the temperature's day from the history, raw or in buckets", () => {
  const t0 = ms('2026-10-18T14:00');
  const h = 3_600_000;
  const raw = {
    from: t0,
    to: t0 + 24 * h,
    before: [t0 - h, 15],
    raw: [
      [t0 + 2 * h, 12],
      [t0 + 16 * h, 5],
      [t0 + 23 * h, 16],
      [t0 + 24 * h, 14],
    ],
  };
  const day = temperatureDay(raw);
  assert.equal(day.points.length, 5);
  assert.deepEqual(day.points[0], [t0, 15]);
  assert.deepEqual(day.min, { ts: t0 + 16 * h, v: 5 });
  assert.deepEqual(day.max, { ts: t0 + 23 * h, v: 16 });
  assert.equal(day.delta, -1);
  const bucketed = temperatureDay({ from: t0, to: t0 + 24 * h, before: null, buckets: [{ ts: t0, avg: 10 }, { ts: t0 + h, avg: null }, { ts: t0 + 2 * h, avg: 12 }] });
  assert.deepEqual(bucketed.points, [
    [t0, 10],
    [t0 + 2 * h, 12],
  ]);
  assert.equal(temperatureDay({ raw: [[t0, 3]] }), null);
  assert.equal(temperatureDay(null), null);
});

test('the spark line spans the box, the lowest value at the bottom', () => {
  const { line, fill } = sparkPath(
    [
      [0, 5],
      [50, 15],
      [100, 10],
    ],
    200,
    100,
    0,
    100,
    10,
  );
  assert.equal(line, 'M0.0 90.0 L100.0 10.0 L200.0 50.0');
  assert.ok(fill.endsWith('L200.0 100 L0.0 100 Z'));
  assert.deepEqual(sparkPath([], 10, 10, 0, 1), { line: '', fill: '' });
});
