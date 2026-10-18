// Which pages of the family's menu this house needs: each one says what
// justifies it (a TV for the living room, a meter for the energy…), read
// from the devices and the layout. Without it, the page leaves the menu but
// its route still answers (`#/energie` typed shows the page, empty). A
// person can force a page in or out (`home.json` `pages`). Pure: tested by
// `pages.test.js`, fed by `home.svelte.js`.

/** The pages a person may show or hide, in the menu's order (the server
 *  accepts these only: `PAGES` of `crates/moli-api/src/home.rs`). */
export const PAGE_IDS = ['pieces', 'salon', 'plan', 'carte', 'cameras', 'parler', 'energie', 'solaire', 'impression', 'dehors', 'automatismes', 'infra', 'moli'];

/** What a person may choose for a page. */
export const PAGE_CHOICES = ['auto', 'always', 'never'];

const has = (d, key, write = false) => (d.points ?? []).some((p) => p.key === key && (!write || p.access?.write));

/** A voice satellite (ESPHome): it listens, thinks, speaks. */
const satellite = (d) => (d.points ?? []).some((p) => p.key === 'state' && p.kind?.values?.includes('listening'));
const printer = (d) => d.printer === true || (has(d, 'progress') && has(d, 'nozzle_temperature'));
const PRODUCTION = /solar|solaire|pv_|photovolta|production/i;

/**
 * What justifies each page here: `{ salon: true, energie: false, … }`.
 * `facts`: `devices` (the hub's, hidden ones left out), `kindOf` (the
 * dashboard's `deviceKind`), `config` (`home.json`), `plan` (`/api/plan`),
 * `energy` (`{ available, meters }` from `/api/energy`), `people`
 * (`{ zones, phones }`), `voice` (Moli speaks or listens).
 */
export function justified({ devices = [], kindOf = () => 'unknown', config = {}, plan = null, energy = null, people = null, voice = false } = {}) {
  const cfg = config ?? {};
  const any = (test) => devices.some(test);
  const production =
    any((d) => (d.points ?? []).some((p) => ['power', 'energy'].includes(p.semantic) && PRODUCTION.test(`${p.key} ${p.label ?? ''}`))) ||
    (energy?.meters ?? []).some((m) => PRODUCTION.test(`${m.role ?? ''} ${m.id ?? ''}`));
  return {
    pieces: true,
    salon: !!cfg.salon || any((d) => kindOf(d) === 'tv'),
    plan: (plan?.floors?.length ?? 0) > 0,
    carte: (people?.zones?.length ?? 0) > 0 || (people?.phones?.length ?? 0) > 0,
    cameras: (cfg.favorites?.cameras?.length ?? 0) > 0 || any((d) => d.camera || kindOf(d) === 'camera'),
    parler: !!voice || any(satellite),
    energie: !!energy?.available || !!cfg.energy,
    solaire: production,
    impression: any(printer),
    dehors: !!cfg.outdoor?.weather || any((d) => has(d, 'weather_code')),
    automatismes: true,
    // The house's machines: an Atelier matter, unless a person asks.
    infra: false,
    moli: true,
  };
}

/** Whether the page shows in the menu: the person's choice, else the house. */
export function shown(id, auto, choices) {
  const choice = choices?.[id];
  if (choice === 'always') return true;
  if (choice === 'never') return false;
  return !!auto[id];
}

/** `{ id: true|false }` for every page. */
export function visiblePages(facts, choices) {
  const auto = justified(facts);
  return Object.fromEntries(PAGE_IDS.map((id) => [id, shown(id, auto, choices)]));
}
