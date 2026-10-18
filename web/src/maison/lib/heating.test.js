import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalize, zoneAt, stretches, moveBoundary, paint, copyDay, clock, nudge, draftOf, DAY, WEEK } from './heating.js';

const week = [
  { zone: 1, start: 0 },
  { zone: 0, start: 390 },
  { zone: 1, start: 1320 },
  { zone: 0, start: DAY + 420 },
];

test('a week is made sound', () => {
  assert.deepEqual(normalize([{ zone: 0, start: 30 }, { zone: 0, start: 60 }, { zone: 1, start: 90 }, { zone: 1, start: WEEK }]), [
    { zone: 0, start: 0 },
    { zone: 1, start: 90 },
  ]);
});

test('a day is cut into stretches', () => {
  assert.equal(zoneAt(week, 400), 0);
  assert.equal(zoneAt(week, DAY + 10), 1);
  assert.deepEqual(stretches(week, 1), [
    { from: DAY, to: DAY + 420, zone: 1, slot: -1 },
    { from: DAY + 420, to: 2 * DAY, zone: 0, slot: 3 },
  ]);
  assert.equal(stretches(week, 0)[0].slot, 0);
});

test('a boundary moves by quarter hours between its neighbours', () => {
  assert.equal(moveBoundary(week, 1, 403)[1].start, 405);
  assert.equal(moveBoundary(week, 1, 5)[1].start, 15);
  assert.equal(moveBoundary(week, 1, 5000)[1].start, 1305);
  assert.equal(moveBoundary(week, 0, 100), week, 'Monday 0:00 stays');
});

test('a stretch is painted, what follows goes on', () => {
  const painted = paint(week, 720, 840, 5);
  assert.deepEqual(painted.slice(1, 4), [
    { zone: 0, start: 390 },
    { zone: 5, start: 720 },
    { zone: 0, start: 840 },
  ]);
  // Painting the same zone over its neighbour merges them.
  assert.deepEqual(paint(week, 1320, 1440, 0).slice(0, 3), [
    { zone: 1, start: 0 },
    { zone: 0, start: 390 },
    { zone: 1, start: DAY },
  ]);
});

test('a day is copied onto others', () => {
  const copied = copyDay(week, 0, [1, 2]);
  for (const day of [1, 2]) {
    assert.deepEqual(
      stretches(copied, day).map((s) => [s.from - day * DAY, s.to - day * DAY, s.zone]),
      [
        [0, 390, 1],
        [390, 1320, 0],
        [1320, DAY, 1],
      ],
    );
  }
  // Wednesday on: as before (comfort since Tuesday 7:00).
  assert.equal(zoneAt(copied, 3 * DAY + 10), 0);
});

test('times and temperatures read well', () => {
  assert.equal(clock(390), '6:30');
  assert.equal(clock(DAY + 5), '0:05');
  assert.equal(clock(DAY), '0:00');
  assert.equal(clock(DAY, true), '24:00');
  assert.equal(nudge(29.5, 1), 30);
  assert.equal(nudge(7, -0.5), 7);
  assert.equal(nudge(19.3, 0.5), 20);
});

test('a draft has each room in each zone', () => {
  const draft = draftOf(
    { id: 's', name: 'S', hg_temp: 7, away_temp: 12, zones: [{ id: 0, name: '', type: 0, rooms: [{ id: 'a', temp: 20 }] }], timetable: [{ zone: 0, start: 0 }] },
    [{ id: 'a' }, { id: 'b' }],
  );
  assert.deepEqual(draft.zones[0].rooms, [
    { id: 'a', temp: 20 },
    { id: 'b', temp: 20 },
  ]);
});
