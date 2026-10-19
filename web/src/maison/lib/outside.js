// Outside, in numbers: the sun's day, the outdoor spaces of the plan and a
// picture of them, the outdoor temperature's last day. Pure: tested by
// `outside.test.js`, fed by `pages/Dehors.svelte`.

import { area, bbox, inside } from '../plan/geometry.js';

/** The kinds of room that are outside (as `plan/scene3d.js` sees them). */
export const OUTDOOR = new Set(['garden', 'deck', 'pool', 'terrace', 'driveway']);

const DAY = 86_400_000;

/** Open-Meteo's local time (« 2026-10-19T08:12 ») as a timestamp, read in
 *  the browser's zone: the house's, where the dashboard is looked at. */
function at(iso) {
  const ms = Date.parse(String(iso ?? ''));
  return Number.isFinite(ms) ? ms : null;
}

/**
 * The sun's day from the weather's `sunrise` and `sunset`: `phase`
 * (`before` dawn, `day`, `after` dusk), `pos` along the arc (0 at sunrise,
 * 1 at sunset), `length` of the day, `until` the next event (ms; after
 * dusk, tomorrow's sunrise taken as today's plus a day), `noon` when the
 * sun is highest. Null without both times.
 */
export function sunFacts(sunrise, sunset, now) {
  const rise = at(sunrise);
  const set = at(sunset);
  if (rise == null || set == null || set <= rise) return null;
  const length = set - rise;
  const phase = now < rise ? 'before' : now <= set ? 'day' : 'after';
  const pos = Math.min(1, Math.max(0, (now - rise) / length));
  const until = phase === 'before' ? rise - now : phase === 'day' ? set - now : rise + DAY - now;
  return { rise, set, noon: rise + length / 2, length, phase, pos, until };
}

/** A duration as whole hours and minutes: `{ h, m }`. */
export function hoursMinutes(ms) {
  const minutes = Math.max(0, Math.round(ms / 60_000));
  return { h: Math.floor(minutes / 60), m: minutes % 60 };
}

/** The drawn floor at ground level (the lowest), where the outside is. */
export function groundFloor(plan) {
  const drawn = (plan?.floors ?? []).filter((f) => f.size && f.rooms?.length);
  if (!drawn.length) return null;
  return drawn.reduce((a, b) => (Number(b.elevation ?? 0) < Number(a.elevation ?? 0) ? b : a));
}

const drawable = (r) => r.kind !== 'void' && r.kind !== 'roof' && Array.isArray(r.poly) && r.poly.length >= 3;

/** The outdoor spaces of a floor, largest first: `[{ id, name, kind, area }]` (m²). */
export function outdoorSpaces(floor) {
  return (floor?.rooms ?? [])
    .filter((r) => drawable(r) && OUTDOOR.has(r.kind))
    .map((r) => ({ id: r.id, name: r.name, kind: r.kind, area: area(r.poly) }))
    .sort((a, b) => b.area - a.area);
}

/**
 * The floor as a small picture: its rooms as polygons, turned a quarter
 * when the plot is taller than wide (a long garden runs across the card),
 * each with the middle of its box for a label, and the devices placed on
 * it; `w` × `h` the drawing's size (cm), `pad` around it.
 */
export function miniPlan(floor, pad = 20) {
  const rooms = (floor?.rooms ?? []).filter(drawable);
  if (!rooms.length) return null;
  const all = bbox(rooms.flatMap((r) => r.poly));
  const turned = all.h > all.w * 1.25;
  const map = turned ? ([x, y]) => [y - all.y, all.w - (x - all.x)] : ([x, y]) => [x - all.x, y - all.y];
  const w = turned ? all.h : all.w;
  const h = turned ? all.w : all.h;
  const shapes = rooms.map((r) => {
    const poly = r.poly.map(map);
    const b = bbox(poly);
    return { id: r.id, name: r.name, kind: r.kind, outdoor: OUTDOOR.has(r.kind), poly, cx: b.x + b.w / 2, cy: b.y + b.h / 2, w: b.w, h: b.h };
  });
  const devices = (floor.devices ?? []).map((d) => {
    const [x, y] = map([d.x, d.y]);
    return { id: d.id, x, y };
  });
  // The house itself: the box around the rooms that are inside.
  const indoors = shapes.filter((r) => !r.outdoor);
  const house = indoors.length ? bbox(indoors.flatMap((r) => r.poly)) : null;
  return {
    rooms: shapes,
    devices,
    w,
    h,
    pad,
    turned,
    house: house ? { cx: house.x + house.w / 2, cy: house.y + house.h / 2, w: house.w, h: house.h } : null,
    viewBox: `${-pad} ${-pad} ${w + 2 * pad} ${h + 2 * pad}`,
  };
}

/** Whether the plan places a device inside a room that is indoors. */
export function placedIndoors(plan, id) {
  for (const floor of plan?.floors ?? []) {
    const spot = (floor.devices ?? []).find((d) => d.id === id);
    if (!spot) continue;
    const rooms = (floor.rooms ?? []).filter(drawable);
    const around = rooms.filter((r) => inside(r.poly, spot.x, spot.y));
    if (!around.length) return false;
    // The smallest room around it, as the drawing counts.
    const room = around.reduce((a, b) => (area(b.poly) < area(a.poly) ? b : a));
    return !OUTDOOR.has(room.kind);
  }
  return false;
}

/**
 * The last day of a numeric point from a history `Series`: `points`
 * `[[ts, value]…]` (the value before the window first, at its start),
 * the coldest and warmest moments, `delta` between now and a day ago.
 * Null without two points.
 */
export function temperatureDay(series) {
  if (!series) return null;
  const numeric = (v) => typeof v === 'number' && Number.isFinite(v);
  let points = series.raw?.length ? series.raw.filter(([, v]) => numeric(v)) : (series.buckets ?? []).filter((b) => numeric(b.avg)).map((b) => [b.ts, b.avg]);
  const from = Number(series.from);
  if (series.before && numeric(series.before[1]) && Number.isFinite(from) && (!points.length || points[0][0] > from)) {
    points = [[from, series.before[1]], ...points];
  }
  if (points.length < 2) return null;
  let min = points[0];
  let max = points[0];
  for (const p of points) {
    if (p[1] < min[1]) min = p;
    if (p[1] > max[1]) max = p;
  }
  const first = points[0];
  const last = points.at(-1);
  return { points, min: { ts: min[0], v: min[1] }, max: { ts: max[0], v: max[1] }, first: first[1], last: last[1], delta: last[1] - first[1], from: first[0], to: last[0] };
}

/**
 * An SVG path through `points` in a `w` × `h` box: time across (from
 * `from` to `to`), value up (the lowest at the bottom, with `margin`
 * above and below). `{ line, fill }`: the stroke, and the area under it.
 */
export function sparkPath(points, w, h, from, to, margin = 8) {
  if (!points?.length) return { line: '', fill: '' };
  const values = points.map(([, v]) => v);
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  const span = Math.max(1, to - from);
  const range = Math.max(1, hi - lo);
  const x = (ts) => ((ts - from) / span) * w;
  const y = (v) => h - margin - ((v - lo) / range) * (h - 2 * margin);
  const coords = points.map(([ts, v]) => `${x(ts).toFixed(1)} ${y(v).toFixed(1)}`);
  const line = `M${coords.join(' L')}`;
  const fill = `${line} L${x(points.at(-1)[0]).toFixed(1)} ${h} L${x(points[0][0]).toFixed(1)} ${h} Z`;
  return { line, fill, x, y };
}
