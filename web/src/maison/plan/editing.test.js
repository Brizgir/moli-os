import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  toGrid, snapValue, snapPoint, snapSpan, clampPoint, clampBox,
  straight, unturned, ends, wallGuides, roomGuides, mergeGuides, newWall, moveWall, stretchWall, setThickness, setLength,
  distance, nearestWall, fitOpening, widthOf, placeOpening, resizeOpening, openingsOn, carryOpening,
  rectRoom, insertVertex, removeVertex, freshId,
  history, remember, settle, undo, redo,
} from './editing.js';

// A box of 600 × 400, walls 20 thick, outside faces on 0 and 600 / 400.
const walls = [
  { x: 0, y: 0, w: 600, h: 20 },
  { x: 0, y: 380, w: 600, h: 20 },
  { x: 0, y: 0, w: 20, h: 400 },
  { x: 580, y: 0, w: 20, h: 400 },
  { x: 100, y: 100, w: 200, h: 15, r: 30 },
];

test('the grid and the lines around catch what moves', () => {
  assert.equal(toGrid(12.4), 10);
  assert.equal(toGrid(12.6), 15);
  assert.equal(snapValue(103, [101, 140], 5), 101, 'a line close by wins');
  assert.equal(snapValue(118, [101, 140], 5), 120, 'else the grid');
  assert.deepEqual(snapPoint([198, 303], { xs: [200], ys: [] }, 6), [200, 305]);
  // The span's far edge lands on 580: it starts at 480.
  assert.equal(snapSpan(483, 100, [580], 6), 480);
  assert.equal(snapSpan(43, 100, [580], 6), 45);
  assert.deepEqual(clampPoint([-300, 50], [600, 400]), [-100, 50]);
  assert.deepEqual(clampBox({ x: 680, y: 0, w: 50, h: 10 }, [600, 400]), { x: 650, y: 0, w: 50, h: 10 });
});

test('a wall turned by a quarter is a plain box; at an angle, it is not', () => {
  assert.deepEqual(straight({ x: 0, y: 0, w: 100, h: 20, r: 90 }), { x: 40, y: -40, w: 20, h: 100 });
  assert.deepEqual(straight({ x: 0, y: 0, w: 100, h: 20, r: -180 }), { x: 0, y: 0, w: 100, h: 20 });
  assert.equal(straight(walls[4]), null);
  assert.deepEqual(unturned({ x: 0, y: 0, w: 100, h: 20, r: 90, height: 100 }), { x: 40, y: -40, w: 20, h: 100, height: 100 });
  assert.equal(unturned(walls[4]), walls[4]);
  assert.deepEqual(ends(walls[2]), [[10, 0], [10, 400]]);
  const g = wallGuides(walls, 0);
  assert.ok(!g.ys.includes(20) || g.ys.filter((y) => y === 20).length === 0, 'the wall moving is not its own guide');
  assert.equal(g.xs.length, 6, 'three straight walls left, the angled one has no faces');
});

test('a wall moves, its edge catching the walls around', () => {
  const guides = wallGuides(walls);
  const wall = newWall(300, 200, 'y');
  assert.deepEqual(wall, { x: 290, y: 50, w: 20, h: 300 });
  // Its right face comes 3 cm from the inside face of the right wall: it touches.
  assert.equal(moveWall(wall, 267, 0, guides, 6).x, 560);
  assert.equal(moveWall(wall, 101, 0, guides, 6).x, 390, 'nothing close: the grid');
  assert.equal(moveWall(wall, 101.3, 0, null, 6).x, 391.3, 'no magnet: where the finger is');
});

test('a wall grows or shrinks by its ends, the other end stays', () => {
  const guides = wallGuides(walls);
  const wall = { x: 100, y: 200, w: 200, h: 20 };
  assert.deepEqual(stretchWall(wall, 1, [577, 0], guides, 6), { x: 100, y: 200, w: 480, h: 20 });
  assert.deepEqual(stretchWall(wall, 0, [22, 0], guides, 6), { x: 20, y: 200, w: 280, h: 20 });
  assert.deepEqual(stretchWall(wall, 0, [900, 0], guides, 6), { x: 290, y: 200, w: 10, h: 20 }, 'never shorter than 10 cm');
  const upright = { x: 200, y: 50, w: 20, h: 100 };
  assert.deepEqual(stretchWall(upright, 1, [0, 383], guides, 6), { x: 200, y: 50, w: 20, h: 330 });
  assert.deepEqual(setThickness(wall, 10), { x: 100, y: 205, w: 200, h: 10 });
  assert.deepEqual(setThickness(upright, 30), { x: 195, y: 50, w: 30, h: 100 });
  assert.deepEqual(setLength(upright, 250), { x: 200, y: 50, w: 20, h: 250 });
});

test('an opening sits in the closest wall and slides along it', () => {
  assert.equal(distance(walls[0], 50, 10), 0);
  assert.equal(distance(walls[0], 50, 50), 30);
  assert.equal(nearestWall(walls, 300, 60).i, 0);
  assert.equal(nearestWall(walls, 560, 200).i, 3);
  assert.equal(nearestWall([], 0, 0), null);
  const door = { kind: 'door', x: 0, y: 0, w: 90, h: 20 };
  // Dropped near the right wall: it turns with it, as thick as it is.
  assert.deepEqual(placeOpening(walls, door, 90, 553, 202), { kind: 'door', x: 580, y: 155, w: 20, h: 90 });
  // Slid past the end of the top wall: it stays inside it.
  assert.deepEqual(placeOpening(walls, door, 90, 640, 10), { kind: 'door', x: 510, y: 0, w: 90, h: 20 });
  // No wall: it just goes there.
  assert.deepEqual(placeOpening([], door, 90, 100, 100), { kind: 'door', x: 55, y: 90, w: 90, h: 20 });
  assert.deepEqual(fitOpening({ x: 0, y: 0, w: 60, h: 20 }, 90, 30), { x: 0, y: 0, w: 60, h: 20 }, 'never wider than its wall');
});

test('an opening keeps its width along its wall', () => {
  const thick = [{ x: 0, y: 0, w: 50, h: 600 }];
  const narrow = { kind: 'window', x: 0, y: 100, w: 50, h: 40 };
  assert.equal(widthOf(narrow, thick), 40, 'along an upright wall, the width is its height');
  assert.equal(widthOf({ kind: 'window', x: 900, y: 900, w: 30, h: 80 }, thick), 80, 'away from any wall: its long side');
  assert.deepEqual(resizeOpening(thick, narrow, 100), { kind: 'window', x: 0, y: 70, w: 50, h: 100 });
});

test('the openings in a wall follow it', () => {
  const openings = [
    { kind: 'window', x: 200, y: 0, w: 120, h: 20 },
    { kind: 'door', x: 580, y: 100, w: 20, h: 90 },
  ];
  assert.deepEqual(openingsOn(walls[0], openings), [0]);
  assert.deepEqual(openingsOn(walls[3], openings), [1]);
  const before = walls[0];
  const after = { x: 0, y: 50, w: 600, h: 10 };
  assert.deepEqual(carryOpening(openings[0], before, after, 0, 50), { kind: 'window', x: 200, y: 50, w: 120, h: 10 });
  const shorter = { x: 0, y: 0, w: 250, h: 20 };
  assert.deepEqual(carryOpening(openings[0], before, shorter), { kind: 'window', x: 130, y: 0, w: 120, h: 20 }, 'kept inside a shorter wall');
});

test('a room: a new one, corners added and taken away, an id of its own', () => {
  assert.deepEqual(rectRoom(302, 198), [[150, 50], [450, 50], [450, 350], [150, 350]]);
  const sq = [[0, 0], [100, 0], [100, 100], [0, 100]];
  assert.deepEqual(insertVertex(sq, 3), [[0, 0], [100, 0], [100, 100], [0, 100], [0, 50]]);
  assert.deepEqual(insertVertex(sq, 0)[1], [50, 0]);
  assert.deepEqual(removeVertex(sq, 1), [[0, 0], [100, 100], [0, 100]]);
  assert.equal(removeVertex([[0, 0], [1, 0], [0, 1]], 0).length, 3, 'three corners at least');
  assert.equal(insertVertex(Array.from({ length: 64 }, (_, i) => [i, i]), 0).length, 64, '64 at most');
  assert.equal(freshId(['piece', 'piece-2']), 'piece-3');
  assert.equal(freshId([], 'Salle d’eau'), 'salle-d-eau');
  // A corner snaps on its neighbours: a right angle.
  const guides = mergeGuides(roomGuides([{ poly: sq }], [0, 2]), wallGuides([]));
  assert.deepEqual(snapPoint([103, 96], guides, 6), [100, 100]);
  assert.ok(!roomGuides([{ poly: sq }], [0, 2]).xs.includes(undefined));
});

test('undo and redo the last changes', () => {
  const h = history(3);
  remember(h, 'a');
  assert.equal(undo(h, 'b'), 'a');
  assert.equal(redo(h, 'a'), 'b');
  assert.equal(redo(h, 'b'), null);
  // A tap that changed nothing leaves what could be redone.
  assert.equal(undo(h, 'b'), 'a');
  remember(h, 'a');
  settle(h, 'a');
  assert.equal(redo(h, 'a'), 'b');
  // A real change forgets it.
  assert.equal(undo(h, 'b'), 'a');
  remember(h, 'a');
  settle(h, 'c');
  assert.equal(redo(h, 'c'), null);
  // Only the last ones are kept.
  for (const s of ['1', '2', '3', '4']) remember(h, s);
  assert.equal(h.past.length, 3);
  assert.equal(undo(h, '5'), '4');
});
