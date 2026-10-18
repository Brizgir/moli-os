// A week of heating, as Netatmo keeps it: from minute `start` (Monday 0:00
// is minute 0) the `zone` applies, until the next slot. These helpers edit
// such a week without ever breaking it: sorted, starting on Monday 0:00,
// two neighbours never the same zone, every start on a quarter hour.

export const DAY = 24 * 60;
export const WEEK = 7 * DAY;
/** A boundary moves by quarter hours. */
export const STEP = 15;
/** Netatmo's bounds for a temperature (°C), by halves. */
export const MIN_TEMP = 7;
export const MAX_TEMP = 30;

const snap = (minute) => Math.round(minute / STEP) * STEP;
const clamp = (x, lo, hi) => Math.min(hi, Math.max(lo, x));

/** The week made sound: sorted, from Monday 0:00, merged. */
export function normalize(timetable) {
  const sorted = [...timetable].map((s) => ({ zone: s.zone, start: s.start })).sort((a, b) => a.start - b.start);
  const out = [];
  for (const slot of sorted) {
    if (slot.start < 0 || slot.start >= WEEK) continue;
    const last = out.at(-1);
    if (last && last.start === slot.start) last.zone = slot.zone;
    else out.push(slot);
  }
  if (!out.length) return out;
  out[0].start = 0;
  return out.filter((s, i) => i === 0 || s.zone !== out[i - 1].zone);
}

/** The zone at `minute` of the week. */
export function zoneAt(timetable, minute) {
  let zone = timetable[0]?.zone ?? null;
  for (const s of timetable) {
    if (s.start > minute) break;
    zone = s.zone;
  }
  return zone;
}

/** One day's stretches: `{ from, to, zone, slot }`, minutes of the week;
 *  `slot` is the index of the slot that starts it, -1 when it began the
 *  day before. */
export function stretches(timetable, day) {
  const from = day * DAY;
  const to = from + DAY;
  const out = [];
  let zone = zoneAt(timetable, from);
  let start = from;
  let slot = timetable.findIndex((s) => s.start === from);
  timetable.forEach((s, i) => {
    if (s.start <= from || s.start >= to) return;
    out.push({ from: start, to: s.start, zone, slot });
    zone = s.zone;
    start = s.start;
    slot = i;
  });
  out.push({ from: start, to, zone, slot });
  return out;
}

/** Slot `index` (not the first) moved to `minute`, kept between its
 *  neighbours by a quarter hour at least. */
export function moveBoundary(timetable, index, minute) {
  if (index <= 0 || index >= timetable.length) return timetable;
  const lo = timetable[index - 1].start + STEP;
  const hi = (timetable[index + 1]?.start ?? WEEK) - STEP;
  if (lo > hi) return timetable;
  const out = timetable.map((s) => ({ ...s }));
  out[index].start = clamp(snap(minute), lo, hi);
  return out;
}

/** `zone` from `from` to `to` (minutes of the week): after `to`, what was
 *  there before goes on. */
export function paint(timetable, from, to, zone) {
  from = clamp(snap(from), 0, WEEK);
  to = clamp(snap(to), 0, WEEK);
  if (to <= from) return normalize(timetable);
  const after = zoneAt(timetable, to);
  const kept = timetable.filter((s) => s.start < from || s.start > to);
  kept.push({ zone, start: from });
  if (to < WEEK && !kept.some((s) => s.start === to)) kept.push({ zone: after, start: to });
  return normalize(kept);
}

/** Day `source`'s stretches laid over each day of `targets`. */
export function copyDay(timetable, source, targets) {
  let out = normalize(timetable);
  const pattern = stretches(out, source).map((s) => ({ from: s.from - source * DAY, to: s.to - source * DAY, zone: s.zone }));
  for (const day of targets) {
    if (day === source) continue;
    for (const p of pattern) out = paint(out, day * DAY + p.from, day * DAY + p.to, p.zone);
  }
  return out;
}

/** « 6:30 » for a minute of the week; `end`: a stretch's end, where
 *  midnight is the day's 24:00. */
export function clock(minuteOfWeek, end = false) {
  let m = ((minuteOfWeek % DAY) + DAY) % DAY;
  if (end && m === 0 && minuteOfWeek > 0) m = DAY;
  return `${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}`;
}

/** A temperature moved by `delta`, kept within Netatmo's bounds by halves. */
export function nudge(temp, delta) {
  return clamp(Math.round(((Number(temp) || 19) + delta) * 2) / 2, MIN_TEMP, MAX_TEMP);
}

/** Netatmo's kind of zone, as words the family uses (a catalogue key). */
export function kindKey(type) {
  return { 0: 'confort', 1: 'nuit', 5: 'eco', 8: 'confort_plus' }[type] ?? 'autre';
}

/** A zone's colour: by kind, then by its rank for custom ones. */
export function zoneColor(zone, index) {
  const byKind = { 0: '#f0a53a', 1: '#4b74f2', 5: '#3fae7a', 8: '#e8684a' };
  const others = ['#9b6fe0', '#2bb3c0', '#d4579b', '#8d9a3b', '#c98a4b'];
  return byKind[zone?.type] ?? others[index % others.length];
}

/** A schedule ready to edit: each heated room in each zone (the median of
 *  the zone's others, else 19 °C, when Netatmo said nothing for it). */
export function draftOf(schedule, rooms) {
  const zones = schedule.zones.map((z) => {
    const temps = z.rooms.map((r) => r.temp).sort((a, b) => a - b);
    const fill = temps.length ? temps[Math.floor(temps.length / 2)] : 19;
    return {
      id: z.id,
      name: z.name,
      type: z.type,
      rooms: [...z.rooms.map((r) => ({ ...r })), ...rooms.filter((r) => !z.rooms.some((x) => x.id === r.id)).map((r) => ({ id: r.id, temp: fill }))],
    };
  });
  return {
    id: schedule.id,
    name: schedule.name,
    hg_temp: schedule.hg_temp,
    away_temp: schedule.away_temp,
    zones,
    timetable: normalize(schedule.timetable),
  };
}
