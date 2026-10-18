// The day's timed automations (« at 21:00 »): what already ran, what comes,
// and the next one that changes the heating. Pure: tested by
// `today.test.js`; the list comes from `GET /api/automations`.

/** Monday 1 … Sunday 7, as the automations count the days. */
export const weekday = (date) => ((date.getDay() + 6) % 7) + 1;

const minutesOf = (at) => {
  const m = /^(\d{1,2}):(\d{2})$/.exec(String(at ?? '').trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
};

/** The points an automation sets (`set`, `toggle`). */
const pointsSet = (a) => (a.graph?.nodes ?? []).filter((n) => n.type === 'set' || n.type === 'toggle').map((n) => n.point).filter(Boolean);

/**
 * The live automations (approved, switched on) with a time of day, on the
 * day `offset` days after `now`: `[{ id, name, at, minutes, done, heating }]`
 * by time. `done`: already past today. `heating(point)`: whether a point it
 * sets is the heating's.
 */
export function timedOn(versions, now, heating = () => false, offset = 0) {
  const day = new Date(now);
  day.setDate(day.getDate() + offset);
  const today = weekday(day);
  const nowMinutes = now.getHours() * 60 + now.getMinutes();
  const out = [];
  for (const v of versions ?? []) {
    const a = v.automation ?? v;
    if (!v.approved || !a.enabled) continue;
    for (const n of a.graph?.nodes ?? []) {
      if (n.type !== 'at_time') continue;
      const minutes = minutesOf(n.at);
      if (minutes == null) continue;
      if (n.days?.length && !n.days.includes(today)) continue;
      out.push({
        id: a.id,
        name: a.name,
        at: n.at,
        minutes,
        done: offset === 0 && minutes <= nowMinutes,
        heating: pointsSet(a).some(heating),
      });
    }
  }
  return out.sort((x, y) => x.minutes - y.minutes || String(x.name).localeCompare(String(y.name)));
}

/** The next heating change: today's still to come, else tomorrow's first.
 *  `{ name, minutes, tomorrow }` or null. */
export function nextHeating(versions, now, heating) {
  const later = timedOn(versions, now, heating, 0).find((x) => x.heating && !x.done);
  if (later) return { name: later.name, minutes: later.minutes, tomorrow: false };
  const first = timedOn(versions, now, heating, 1).find((x) => x.heating);
  return first ? { name: first.name, minutes: first.minutes, tomorrow: true } : null;
}
