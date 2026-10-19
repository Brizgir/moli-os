// A heated room's last hours or days, from the history (`/api/history`,
// recorded on change): the temperature it read (a line), the setpoint it
// aimed at (steps: it changes by stretches) and when it asked for heat
// (intervals, drawn as a band). Pure: tested by `heating-curves.test.js`,
// drawn by `ui/HeatingCurves.svelte`.
//
// A `Series` is `{ before, raw:[[ts, v]…], buckets:[{ts, avg, min, max}] }`:
// `before` is the value held when the window opens, `raw` every change,
// `buckets` (when there are too many changes) averages per stretch. A
// boolean is bucketed as 0 / 1.

const HOUR = 3_600_000;
const DAY_MS = 24 * HOUR;

/** The periods a person may choose, in hours. */
export const RANGES = { day: 24, week: 7 * 24, month: 30 * 24 };

const isNum = (v) => typeof v === 'number' && Number.isFinite(v);
const asNum = (v) => (typeof v === 'boolean' ? Number(v) : isNum(v) ? v : null);

/** The changes of a series in the window, numbers or null (no reading). */
function changes(series, from, to) {
  if (!series) return [];
  const list = series.buckets?.length ? series.buckets.map((b) => [b.ts, b.avg]) : (series.raw ?? []).map(([ts, v]) => [ts, asNum(v)]);
  return list.filter(([ts]) => ts >= from && ts <= to);
}

const held = (series) => (series?.before ? asNum(series.before[1]) : null);

/** A measured line `[[ts, v]…]` over the window: from the value held when
 *  it opens to the last one, held until `to` (nothing changed since). A
 *  null `v` is a gap (the device said nothing). */
export function line(series, from, to) {
  const points = changes(series, from, to);
  const start = held(series);
  if (start != null && (!points.length || points[0][0] > from)) points.unshift([from, start]);
  const last = points.at(-1);
  if (last && last[1] != null && last[0] < to) points.push([to, last[1]]);
  return points;
}

/** A value by stretches, `[{ from, to, value }]`: what held when the window
 *  opens, then each change until the next one. Stretches without a value
 *  are left out; neighbours with the same value are merged. */
export function steps(series, from, to) {
  const points = changes(series, from, to);
  const start = held(series);
  if (start != null && (!points.length || points[0][0] > from)) points.unshift([from, start]);
  const out = [];
  points.forEach(([ts, value], i) => {
    const end = points[i + 1]?.[0] ?? to;
    if (value == null || end <= ts) return;
    const prev = out.at(-1);
    if (prev && prev.to === ts && prev.value === value) prev.to = end;
    else out.push({ from: ts, to: end, value });
  });
  return out;
}

/** When it heated, `[{ from, to, share }]`: `share` 1 all along, less for a
 *  long window's stretch where it heated part of the time (a bucket that
 *  saw both: its share of « on » changes). */
export function heats(series, from, to) {
  if (!series) return [];
  const out = [];
  const add = (a, b, share) => {
    if (b <= a || share <= 0) return;
    const prev = out.at(-1);
    if (prev && prev.to === a && prev.share === share) prev.to = b;
    else out.push({ from: a, to: b, share });
  };
  const buckets = (series.buckets ?? []).filter((b) => b.ts >= from && b.ts <= to);
  const first = buckets[0]?.ts ?? series.raw?.find(([ts]) => ts >= from)?.[0] ?? to;
  if (held(series) > 0) add(from, first, 1);
  if (buckets.length) {
    buckets.forEach((b, i) => {
      const end = buckets[i + 1]?.ts ?? to;
      add(b.ts, end, b.min >= 1 ? 1 : b.max <= 0 ? 0 : Math.round(b.avg * 100) / 100);
    });
    return out;
  }
  for (const s of steps({ raw: series.raw }, from, to)) add(s.from, s.to, s.value > 0 ? 1 : 0);
  return out;
}

/** The last reading of a series: `[ts, v]` (the last change, else the value
 *  held before the window), or null. */
export function latest(series) {
  if (!series) return null;
  const raw = (series.raw ?? []).filter(([, v]) => asNum(v) != null);
  const last = series.last && asNum(series.last[1]) != null ? series.last : raw.at(-1) ?? null;
  if (last) return [last[0], asNum(last[1])];
  return series.before && asNum(series.before[1]) != null ? [series.before[0], asNum(series.before[1])] : null;
}

/** A line's value at `ts`, between its two neighbours; null outside it or
 *  in a gap. */
export function valueAt(points, ts) {
  if (!points.length || ts < points[0][0] || ts > points.at(-1)[0]) return null;
  for (let i = 1; i < points.length; i++) {
    const [t1, v1] = points[i];
    if (t1 < ts) continue;
    const [t0, v0] = points[i - 1];
    if (v0 == null || v1 == null) return t1 === ts ? v1 : null;
    return t1 === t0 ? v1 : v0 + ((v1 - v0) * (ts - t0)) / (t1 - t0);
  }
  return points[0][1];
}

/** The stretch holding at `ts` (steps or heats), or null. */
export const stretchAt = (list, ts) => list.find((s) => s.from <= ts && ts < s.to) ?? (list.at(-1)?.to === ts ? list.at(-1) : null);

/** An axis for temperatures: whole bounds around every value (`minSpan`
 *  degrees at least, so a steady room is not a storm), and its ticks. */
export function scale(values, minSpan = 4) {
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of values) {
    if (!isNum(v)) continue;
    lo = Math.min(lo, v);
    hi = Math.max(hi, v);
  }
  if (lo > hi) [lo, hi] = [17, 22];
  if (hi - lo < minSpan) {
    const mid = (lo + hi) / 2;
    [lo, hi] = [mid - minSpan / 2, mid + minSpan / 2];
  }
  const step = [1, 2, 5, 10].find((s) => (hi - lo) / s <= 5) ?? 10;
  lo = Math.floor(lo / step) * step;
  hi = Math.ceil(hi / step) * step;
  const ticks = [];
  for (let v = lo; v <= hi + 1e-9; v += step) ticks.push(v);
  return { lo, hi, ticks };
}

/** Where to mark the time between `from` and `to`, on the house's clock:
 *  round hours for a day, midnights for longer, `max` marks at most.
 *  `[{ ts, day }]`, `day` when it opens a day. */
export function timeTicks(from, to, max = 6) {
  const span = to - from;
  const out = [];
  if (span <= 2 * DAY_MS) {
    const step = [1, 2, 3, 4, 6, 12, 24].find((h) => span / (h * HOUR) <= max) ?? 24;
    const d = new Date(from);
    d.setMinutes(0, 0, 0);
    while (d.getTime() < from || d.getHours() % step) d.setHours(d.getHours() + 1);
    for (; d.getTime() <= to; d.setHours(d.getHours() + step)) out.push({ ts: d.getTime(), day: d.getHours() === 0 });
    return out;
  }
  const step = [1, 2, 3, 5, 7, 10, 15].find((n) => span / (n * DAY_MS) <= max) ?? 15;
  const d = new Date(from);
  d.setHours(0, 0, 0, 0);
  if (d.getTime() < from) d.setDate(d.getDate() + 1);
  for (; d.getTime() <= to; d.setDate(d.getDate() + step)) out.push({ ts: d.getTime(), day: true });
  return out;
}

const f = (n) => Math.round(n * 10) / 10;

/** An SVG path for a line through `x(ts)`, `y(v)`; a gap starts anew. */
export function linePath(points, x, y) {
  let d = '';
  let pen = false;
  for (const [ts, v] of points) {
    if (v == null) {
      pen = false;
      continue;
    }
    d += `${pen ? 'L' : 'M'}${f(x(ts))} ${f(y(v))}`;
    pen = true;
  }
  return d;
}

/** An SVG path for stretches: flat along each, straight down or up where
 *  one follows the other, lifted where there is none. */
export function stepPath(list, x, y) {
  let d = '';
  list.forEach((s, i) => {
    const joined = i > 0 && list[i - 1].to === s.from;
    d += `${joined ? 'V' : 'M'}${joined ? f(y(s.value)) : `${f(x(s.from))} ${f(y(s.value))}`}H${f(x(s.to))}`;
  });
  return d;
}
