// Which TVs a room shows a remote for (pure: tested in tvs.test.js).

/** A device with a remote: it takes key presses (`key`, written). */
export const hasRemote = (d) => !!d?.points?.some((p) => p.key === 'key' && p.access?.write);

/** The TVs of `room`, by `roomOf(d)`: `first` (the layout's choice, if
 *  any) leads, `also` (more ids the layout names) next, then the others by
 *  name. Hidden devices stay out, unless the layout names them. */
export function roomTvs(devices, { room = null, first = null, also = [], roomOf, nameOf, hidden = () => false }) {
  const byId = new Map(devices.map((d) => [d.id, d]));
  const named = [first, ...also].filter((id) => id && byId.has(id));
  const home = room ?? (first && byId.has(first) ? roomOf(byId.get(first)) : null);
  const rest = home
    ? devices
        .filter((d) => hasRemote(d) && !hidden(d.id) && !named.includes(d.id) && roomOf(d) === home)
        .sort((a, b) => nameOf(a.id).localeCompare(nameOf(b.id)))
        .map((d) => d.id)
    : [];
  return [...new Set([...named, ...rest])];
}

/** The TV chosen among `tvs`: the one picked before if it is still there,
 *  else the first. */
export const chosen = (tvs, picked) => (tvs.includes(picked) ? picked : (tvs[0] ?? null));
