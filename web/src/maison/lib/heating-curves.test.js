import { test } from 'node:test';
import assert from 'node:assert/strict';
import { line, steps, heats, latest, valueAt, stretchAt, scale, timeTicks, linePath, stepPath } from './heating-curves.js';

const H = 3_600_000;
const from = 100 * H;
const to = from + 24 * H;

test('a reading is held from the value before the window until now', () => {
  const s = { before: [from - H, 19.2], raw: [[from + 2 * H, 19.6], [from + 5 * H, 20.1]] };
  assert.deepEqual(line(s, from, to), [
    [from, 19.2],
    [from + 2 * H, 19.6],
    [from + 5 * H, 20.1],
    [to, 20.1],
  ]);
  // A long window comes in buckets: their averages.
  const b = { before: null, buckets: [{ ts: from, avg: 18, min: 17.5, max: 18.5, count: 9 }, { ts: from + 12 * H, avg: 19, min: 19, max: 19, count: 3 }] };
  assert.deepEqual(line(b, from, to), [
    [from, 18],
    [from + 12 * H, 19],
    [to, 19],
  ]);
  // A null is a gap, never held on.
  assert.deepEqual(line({ raw: [[from + H, 20], [from + 2 * H, null]] }, from, to), [
    [from + H, 20],
    [from + 2 * H, null],
  ]);
  assert.deepEqual(line(null, from, to), []);
});

test('a setpoint goes by stretches, from the one before the window', () => {
  const s = { before: [from - 5 * H, 17], raw: [[from + 6 * H, 20], [from + 7 * H, 20], [from + 22 * H, 17]] };
  assert.deepEqual(steps(s, from, to), [
    { from, to: from + 6 * H, value: 17 },
    { from: from + 6 * H, to: from + 22 * H, value: 20 },
    { from: from + 22 * H, to, value: 17 },
  ]);
  // Nothing changed: what held, all along.
  assert.deepEqual(steps({ before: [from - H, 19], raw: [] }, from, to), [{ from, to, value: 19 }]);
  // Nothing known before the first change.
  assert.deepEqual(steps({ raw: [[from + H, 18], [from + 2 * H, null]] }, from, to), [{ from: from + H, to: from + 2 * H, value: 18 }]);
});

test('heating comes as intervals', () => {
  const s = { before: [from - H, true], raw: [[from + H, false], [from + 3 * H, true], [from + 4 * H, false], [from + 20 * H, true]] };
  assert.deepEqual(heats(s, from, to), [
    { from, to: from + H, share: 1 },
    { from: from + 3 * H, to: from + 4 * H, share: 1 },
    { from: from + 20 * H, to, share: 1 },
  ]);
  assert.deepEqual(heats({ before: [from - H, false], raw: [] }, from, to), []);
  // Buckets: all on, all off, or a share of the stretch.
  const b = {
    before: [from - H, true],
    buckets: [
      { ts: from + H, avg: 1, min: 1, max: 1, count: 1 },
      { ts: from + 2 * H, avg: 1, min: 1, max: 1, count: 1 },
      { ts: from + 3 * H, avg: 0.5, min: 0, max: 1, count: 4 },
      { ts: from + 4 * H, avg: 0, min: 0, max: 0, count: 1 },
    ],
  };
  assert.deepEqual(heats(b, from, to), [
    { from, to: from + 3 * H, share: 1 },
    { from: from + 3 * H, to: from + 4 * H, share: 0.5 },
  ]);
});

test('the last reading, else the one before', () => {
  assert.deepEqual(latest({ last: [from + H, 20.5], raw: [[from + H, 20.5]] }), [from + H, 20.5]);
  assert.deepEqual(latest({ before: [from - H, 19], last: null, raw: [] }), [from - H, 19]);
  assert.equal(latest({ raw: [] }), null);
});

test('values are read at a moment', () => {
  const points = [
    [0, 18],
    [10, 20],
    [20, null],
    [30, 21],
  ];
  assert.equal(valueAt(points, 5), 19);
  assert.equal(valueAt(points, 10), 20);
  assert.equal(valueAt(points, 25), null);
  assert.equal(valueAt(points, 40), null);
  const list = [
    { from: 0, to: 10, value: 17 },
    { from: 10, to: 20, value: 20 },
  ];
  assert.equal(stretchAt(list, 10).value, 20);
  assert.equal(stretchAt(list, 20).value, 20);
  assert.equal(stretchAt(list, 25), null);
});

test('the temperature axis is whole and never too tight', () => {
  assert.deepEqual(scale([19.4, 20.2, null, 19.8]), { lo: 17, hi: 22, ticks: [17, 18, 19, 20, 21, 22] });
  assert.deepEqual(scale([4, 22]), { lo: 0, hi: 25, ticks: [0, 5, 10, 15, 20, 25] });
  assert.ok(scale([]).ticks.length > 1, 'an axis even without readings');
});

test('time is marked on round hours, or on midnights', () => {
  const start = new Date(2026, 9, 12, 13, 25).getTime();
  const day = timeTicks(start, start + 24 * H, 6);
  assert.ok(day.length <= 6 && day.length >= 4);
  for (const t of day) assert.equal(new Date(t.ts).getMinutes(), 0);
  assert.equal(new Date(day[0].ts).getHours() % 4, 0, 'every four hours');
  assert.ok(day.some((t) => t.day));
  const week = timeTicks(start, start + 7 * 24 * H, 8);
  assert.equal(week.length, 7);
  for (const t of week) assert.equal(new Date(t.ts).getHours(), 0);
  const month = timeTicks(start, start + 30 * 24 * H, 6);
  assert.ok(month.length <= 6 && month.length >= 4);
});

test('paths: a line with gaps, stretches joined or lifted', () => {
  const x = (ts) => ts;
  const y = (v) => v;
  assert.equal(linePath([[0, 1], [2, 3], [3, null], [4, 5]], x, y), 'M0 1L2 3M4 5');
  assert.equal(
    stepPath(
      [
        { from: 0, to: 2, value: 1 },
        { from: 2, to: 4, value: 3 },
        { from: 6, to: 8, value: 3 },
      ],
      x,
      y,
    ),
    'M0 1H2V3H4M6 3H8',
  );
});
