// Editing a drawn floor (cm): rooms, walls, openings. Pure: no Svelte, no
// screen; Drawing.svelte and Plan.svelte call it, editing.test.js checks it.
// The kinds mirror crates/moli-api/src/plan.rs (ROOM_KINDS, OPENING_KINDS):
// keep both lists in step.

/** The grid things snap to, cm. */
export const GRID = 5;

export const ROOM_KINDS = [
  'living', 'kitchen', 'bedroom', 'bath', 'wc', 'office', 'hall', 'veranda', 'storage', 'void',
  'roof', 'garden', 'deck', 'pool', 'terrace', 'driveway', 'garage', 'other',
];
export const OPENING_KINDS = ['door', 'window', 'bay', 'gate'];
/** How wide a new opening is, cm. */
export const OPENING_WIDTH = { door: 90, window: 120, bay: 240, gate: 350 };
/** A new wall, cm. */
export const WALL = { thickness: 20, length: 300 };
/** The shortest wall and the narrowest opening, cm. */
export const MIN_WALL = 10;
export const MIN_OPENING = 20;
/** A room has 3 to 64 corners (plan.rs MAX_POINTS). */
export const MAX_POINTS = 64;
/** Shapes may stand this far outside the floor (plan.rs). */
export const MARGIN = 100;

const round = (v) => Math.round(v * 10) / 10;

export const toGrid = (v, grid = GRID) => Math.round(v / grid) * grid;

/** The closest of `targets` within `tol` cm, else the grid (`null`: none, keep `v`). */
export function snapValue(v, targets, tol, grid = GRID) {
  let best = null;
  let gap = tol;
  for (const t of targets) {
    const d = Math.abs(t - v);
    if (d <= gap) {
      gap = d;
      best = t;
    }
  }
  return best ?? (grid ? toGrid(v, grid) : round(v));
}

/** A point on its neighbours' lines (right angles), else on the grid. */
export function snapPoint([x, y], guides, tol) {
  return [snapValue(x, guides.xs, tol), snapValue(y, guides.ys, tol)];
}

/** Where a span [a, a + len] starts once one of its edges sits on a target,
 *  else its start on the grid. */
export function snapSpan(a, len, targets, tol) {
  let best = null;
  let gap = tol;
  for (const t of targets) {
    for (const edge of [a, a + len]) {
      const d = Math.abs(t - edge);
      if (d <= gap) {
        gap = d;
        best = a + (t - edge);
      }
    }
  }
  return best ?? toGrid(a);
}

/** Inside the floor (and its margin). */
export function clampPoint([x, y], [w, h], margin = MARGIN) {
  return [Math.min(w + margin, Math.max(-margin, x)), Math.min(h + margin, Math.max(-margin, y))];
}

/** A rectangle that stays inside the floor (and its margin). */
export function clampBox(b, [w, h], margin = MARGIN) {
  return {
    ...b,
    x: Math.min(w + margin - b.w, Math.max(-margin, b.x)),
    y: Math.min(h + margin - b.h, Math.max(-margin, b.y)),
  };
}

// ---- walls -------------------------------------------------------------------------

/** A wall turned by a quarter (or not at all) as the plain box it covers;
 *  `null` for a wall at an angle. */
export function straight(w) {
  const r = ((((w.r ?? 0) % 360) + 360) % 360);
  if (r === 0 || r === 180) return { x: w.x, y: w.y, w: w.w, h: w.h };
  if (r === 90 || r === 270) {
    const cx = w.x + w.w / 2;
    const cy = w.y + w.h / 2;
    return { x: cx - w.h / 2, y: cy - w.w / 2, w: w.h, h: w.w };
  }
  return null;
}

/** The same wall without its quarter turn (an angled wall stays as it is). */
export function unturned(w) {
  const box = straight(w);
  if (!box) return w;
  const { r, ...rest } = w;
  return { ...rest, ...box };
}

export const horizontal = (b) => b.w >= b.h;
export const lengthOf = (b) => Math.max(b.w, b.h);
export const thicknessOf = (b) => Math.min(b.w, b.h);

/** A wall's two ends, on its middle line. */
export function ends(b) {
  return horizontal(b)
    ? [[b.x, b.y + b.h / 2], [b.x + b.w, b.y + b.h / 2]]
    : [[b.x + b.w / 2, b.y], [b.x + b.w / 2, b.y + b.h]];
}

/** Every straight wall's faces, the snapping lines (`skip`: the wall moving). */
export function wallGuides(walls, skip = -1) {
  const xs = [];
  const ys = [];
  walls.forEach((w, i) => {
    const b = i === skip ? null : straight(w);
    if (!b) return;
    xs.push(b.x, b.x + b.w);
    ys.push(b.y, b.y + b.h);
  });
  return { xs, ys };
}

/** The corners of the rooms (`skip`: [room, corner], the one moving). */
export function roomGuides(rooms, skip = null) {
  const xs = [];
  const ys = [];
  rooms.forEach((r, i) =>
    r.poly.forEach(([x, y], j) => {
      if (skip && skip[0] === i && skip[1] === j) return;
      xs.push(x);
      ys.push(y);
    }),
  );
  return { xs, ys };
}

export const mergeGuides = (...all) => ({ xs: all.flatMap((g) => g.xs), ys: all.flatMap((g) => g.ys) });

/** A new wall centred on (cx, cy), along x or y. */
export function newWall(cx, cy, along = 'x', { length, thickness } = WALL) {
  const [w, h] = along === 'x' ? [length, thickness] : [thickness, length];
  return { x: toGrid(cx - w / 2), y: toGrid(cy - h / 2), w, h };
}

/** A wall moved by (dx, dy), an edge caught by the walls around when close. */
export function moveWall(b, dx, dy, guides, tol) {
  if (!guides) return { ...b, x: round(b.x + dx), y: round(b.y + dy) };
  return { ...b, x: snapSpan(b.x + dx, b.w, guides.xs, tol), y: snapSpan(b.y + dy, b.h, guides.ys, tol) };
}

/** One end of a wall (0: left or top, 1: right or bottom) brought to `p`
 *  along its length; the other end stays. */
export function stretchWall(b, end, [px, py], guides, tol) {
  const flat = horizontal(b);
  const v = flat ? px : py;
  const at = guides ? snapValue(v, flat ? guides.xs : guides.ys, tol) : round(v);
  const [start, size] = flat ? ['x', 'w'] : ['y', 'h'];
  const a = b[start];
  const z = b[start] + b[size];
  const out = { ...b };
  if (end === 0) {
    out[start] = Math.min(at, z - MIN_WALL);
    out[size] = z - out[start];
  } else {
    out[size] = Math.max(at, a + MIN_WALL) - a;
  }
  return out;
}

/** A wall as thick as `t`, on the same middle line. */
export function setThickness(b, t) {
  const v = Math.max(1, t);
  if (horizontal(b)) {
    const cy = b.y + b.h / 2;
    return { ...b, y: round(cy - v / 2), h: v };
  }
  const cx = b.x + b.w / 2;
  return { ...b, x: round(cx - v / 2), w: v };
}

/** A wall as long as `len`, from the same start. */
export function setLength(b, len) {
  const v = Math.max(MIN_WALL, len);
  return horizontal(b) ? { ...b, w: v } : { ...b, h: v };
}

// ---- openings ------------------------------------------------------------------------

/** How far (x, y) is from a box (0 inside). */
export function distance(b, x, y) {
  const dx = Math.max(b.x - x, 0, x - (b.x + b.w));
  const dy = Math.max(b.y - y, 0, y - (b.y + b.h));
  return Math.hypot(dx, dy);
}

/** The straight wall closest to (x, y): `{ i, box, d }`, or `null`. */
export function nearestWall(walls, x, y) {
  let best = null;
  walls.forEach((w, i) => {
    const box = straight(w);
    if (!box) return;
    const d = distance(box, x, y);
    // Two walls meet at a corner: the thinner distance wins, then the longer wall.
    if (!best || d < best.d - 0.01 || (Math.abs(d - best.d) <= 0.01 && lengthOf(box) > lengthOf(best.box))) best = { i, box, d };
  });
  return best;
}

export const centerOf = (b) => [b.x + b.w / 2, b.y + b.h / 2];

/** An opening `width` wide in the wall `wall`, centred at `along` (x for a
 *  wall along x, y otherwise), kept inside the wall. */
export function fitOpening(wall, width, along) {
  const len = lengthOf(wall);
  const wd = Math.max(Math.min(MIN_OPENING, len), Math.min(width, len));
  if (horizontal(wall)) {
    const c = Math.min(wall.x + wall.w - wd / 2, Math.max(wall.x + wd / 2, along));
    return { x: round(c - wd / 2), y: wall.y, w: wd, h: wall.h };
  }
  const c = Math.min(wall.y + wall.h - wd / 2, Math.max(wall.y + wd / 2, along));
  return { x: wall.x, y: round(c - wd / 2), w: wall.w, h: wd };
}

/** How wide an opening is: along its wall if it sits in one. */
export function widthOf(o, walls) {
  const [cx, cy] = centerOf(o);
  const near = nearestWall(walls, cx, cy);
  if (near && near.d <= thicknessOf(near.box)) return horizontal(near.box) ? o.w : o.h;
  return lengthOf(o);
}

/** An opening brought to (x, y): in the closest wall, sliding along it
 *  (on the grid when `snap`). Without a wall, it just moves. */
export function placeOpening(walls, o, width, x, y, snap = true) {
  const near = nearestWall(walls, x, y);
  if (!near) {
    const w = horizontal(o) ? width : o.w;
    const h = horizontal(o) ? o.h : width;
    return { ...o, x: round(x - w / 2), y: round(y - h / 2), w, h };
  }
  const along = horizontal(near.box) ? x : y;
  return { ...o, ...fitOpening(near.box, width, snap ? toGrid(along) : along) };
}

/** An opening `width` wide, on the same wall, around the same middle. */
export function resizeOpening(walls, o, width) {
  const [cx, cy] = centerOf(o);
  const near = nearestWall(walls, cx, cy);
  if (!near) return horizontal(o) ? { ...o, x: round(cx - width / 2), w: width } : { ...o, y: round(cy - width / 2), h: width };
  return { ...o, ...fitOpening(near.box, width, horizontal(near.box) ? cx : cy) };
}

/** The openings that sit in a wall (their middle in it). */
export function openingsOn(wall, openings) {
  const b = straight(wall) ?? wall;
  return openings.flatMap((o, i) => {
    const [cx, cy] = centerOf(o);
    return distance(b, cx, cy) <= 2 ? [i] : [];
  });
}

/** An opening following its wall: moved by (dx, dy), then fitted in the
 *  wall as it is now (its thickness, its ends). */
export function carryOpening(o, wallBefore, wallAfter, dx = 0, dy = 0) {
  const width = horizontal(wallBefore) ? o.w : o.h;
  const [cx, cy] = centerOf(o);
  return { ...o, ...fitOpening(wallAfter, width, horizontal(wallAfter) ? cx + dx : cy + dy) };
}

// ---- rooms -----------------------------------------------------------------------------

/** A new rectangular room centred on (cx, cy). */
export function rectRoom(cx, cy, w = 300, h = 300) {
  const x = toGrid(cx - w / 2);
  const y = toGrid(cy - h / 2);
  return [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
}

/** A corner added in the middle of side `i` (from corner i to i + 1). */
export function insertVertex(poly, i) {
  if (poly.length >= MAX_POINTS) return poly;
  const [x1, y1] = poly[i];
  const [x2, y2] = poly[(i + 1) % poly.length];
  const out = poly.slice();
  out.splice(i + 1, 0, [round((x1 + x2) / 2), round((y1 + y2) / 2)]);
  return out;
}

/** Corner `i` taken away (a room keeps 3 at least). */
export function removeVertex(poly, i) {
  if (poly.length <= 3) return poly;
  return poly.filter((_, j) => j !== i);
}

/** An id like the server wants (a-z, 0-9, -), not taken yet. */
export function freshId(taken, base = 'piece') {
  const stem =
    String(base)
      .normalize('NFD')
      .replace(/\p{M}/gu, '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 24) || 'piece';
  const ids = new Set(taken);
  let id = stem;
  for (let n = 2; ids.has(id); n++) id = `${stem}-${n}`;
  return id;
}

// ---- undo, redo ------------------------------------------------------------------------

/** The last changes of an edit, as snapshots (strings). */
export function history(limit = 50) {
  return { past: [], future: [], stash: null, limit };
}

/** Before a change: what there was. */
export function remember(h, state) {
  h.past.push(state);
  if (h.past.length > h.limit) h.past.shift();
  h.stash = h.future.slice();
  h.future.length = 0;
}

/** After it: nothing changed (a tap, not a drag) forgets it, and what could
 *  be redone still can. */
export function settle(h, state) {
  if (h.past.length && h.past[h.past.length - 1] === state) {
    h.past.pop();
    if (h.stash) h.future.push(...h.stash);
  }
  h.stash = null;
}

/** The state before the last change (`null`: nothing to undo). */
export function undo(h, state) {
  if (!h.past.length) return null;
  h.future.push(state);
  return h.past.pop();
}

export function redo(h, state) {
  if (!h.future.length) return null;
  h.past.push(state);
  return h.future.pop();
}
