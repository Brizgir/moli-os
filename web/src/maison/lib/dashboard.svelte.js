// The home page's arrangement: what the server keeps for the house and for
// the person looking (`/api/dashboard`), which named layout this device
// shows, and the draft while the page is being changed. The pure part
// (default, resolution, editing) is `layout.js`.

import { hub, onDashboardChanged } from '../../lib/hub.svelte.js';
import { t } from '../../lib/i18n.svelte.js';
import { home, savePref } from './home.svelte.js';
import { asHuman } from './auto.svelte.js';
import { resolve, screenFor, defaultLayouts, withCards } from './layout.js';

function readPref(key, fallback) {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export const dashboard = $state({
  /** read once from the server (`null` layouts: nothing kept) */
  loaded: false,
  house: null,
  mine: null,
  canEditHouse: false,
  canEditMine: false,
  /** the named layout this device shows (`localStorage`, like the theme) */
  active: readPref('maison-layout', 'default'),
  /** the screen the page is on, from its width */
  screen: typeof window === 'undefined' ? 'desktop' : screenFor(window.innerWidth),
  /** in the middle of a change: the draft, and what it will be saved as */
  editing: null,
});

const headers = { 'content-type': 'application/json', 'x-moli-origin': 'ui' };

let started = false;

/** Reads the arrangement, and again whenever it is written elsewhere. */
export async function startDashboard() {
  await loadDashboard();
  if (started) return;
  started = true;
  onDashboardChanged(() => {
    if (!dashboard.editing) loadDashboard();
  });
  const update = () => (dashboard.screen = screenFor(window.innerWidth));
  window.addEventListener('resize', update);
  update();
}

export async function loadDashboard() {
  try {
    const res = await fetch('/api/dashboard');
    const body = res.ok ? await res.json() : {};
    dashboard.house = body.house ?? null;
    dashboard.mine = body.mine ?? null;
    dashboard.canEditHouse = !!body.can_edit_house;
    dashboard.canEditMine = !!body.can_edit_mine;
  } catch {
    /* the default, until the link is back */
  } finally {
    dashboard.loaded = true;
  }
}

/** What the page shows now (the draft while editing). */
export function current() {
  if (dashboard.editing) {
    const { set, key } = dashboard.editing;
    return resolve({ mine: set }, dashboard.screen, key);
  }
  return resolve({ house: dashboard.house, mine: dashboard.mine }, dashboard.screen, dashboard.active);
}

/** Whether this viewer may change anything (the button shows). */
export const canEdit = () => dashboard.canEditMine || dashboard.canEditHouse;

export function chooseActive(key) {
  dashboard.active = key;
  savePref('maison-layout', key);
}

/**
 * Opens the draft: a person who has layouts of their own edits them; else
 * the owner edits the house's (a member starts a copy of what they see, as
 * their own). `scope` forces `house` or `mine` when both are allowed.
 */
export function beginEditing(scope = null) {
  const mineFirst = dashboard.canEditMine && (scope !== 'house' || !dashboard.canEditHouse);
  const target = mineFirst ? 'mine' : 'house';
  // Plain copies of what is kept (the state is a proxy): the draft is its own.
  const copy = (layouts) => (layouts ? JSON.parse(JSON.stringify($state.snapshot(layouts))) : null);
  const seen = copy(dashboard.mine) ?? copy(dashboard.house) ?? defaultLayouts();
  const set = target === 'mine' ? (copy(dashboard.mine) ?? seen) : (copy(dashboard.house) ?? defaultLayouts());
  const key = dashboard.active in set ? dashboard.active : 'default';
  dashboard.editing = { scope: target, set, key, history: [], busy: false, error: '' };
}

/** The draft's cards on this screen replaced (one step the person can undo). */
export function draftCards(cards) {
  const e = dashboard.editing;
  if (!e) return;
  e.history = [...e.history.slice(-49), e.set];
  e.set = withCards(e.set, e.key, dashboard.screen, cards);
}

/** The draft's whole set replaced (a named layout added or removed). */
export function draftSet(set, key = null) {
  const e = dashboard.editing;
  if (!e) return;
  e.history = [...e.history.slice(-49), e.set];
  e.set = set;
  if (key) e.key = key;
}

export function undo() {
  const e = dashboard.editing;
  if (!e?.history.length) return;
  e.set = e.history.at(-1);
  e.history = e.history.slice(0, -1);
  if (!(e.key in e.set)) e.key = 'default';
}

export function cancelEditing() {
  dashboard.editing = null;
}

async function call(method, path, body) {
  const res = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const answer = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = new Error(answer.error ?? res.statusText);
    err.status = res.status;
    throw err;
  }
  return answer;
}

/** Saves the draft (the code is asked once if the session has none). */
export async function finishEditing() {
  const e = dashboard.editing;
  if (!e || e.busy) return false;
  e.busy = true;
  e.error = '';
  const path = e.scope === 'house' ? '/api/dashboard/house' : '/api/dashboard/me';
  try {
    const { layouts } = await asHuman(t('maison.modifier.enregistrer_label'), () => call('PUT', path, e.set), 'disposition');
    if (e.scope === 'house') dashboard.house = layouts;
    else dashboard.mine = layouts;
    chooseActive(e.key);
    dashboard.editing = null;
    return true;
  } catch (err) {
    e.error = err.message;
    return false;
  } finally {
    if (dashboard.editing === e) e.busy = false;
  }
}

/** Forgets what is kept for the draft's scope: the page goes back to what
 *  it shows without it (the house's, or the default). */
export async function forgetLayouts() {
  const e = dashboard.editing;
  if (!e || e.busy) return false;
  e.busy = true;
  e.error = '';
  const path = e.scope === 'house' ? '/api/dashboard/house' : '/api/dashboard/me';
  try {
    await asHuman(t('maison.modifier.par_defaut'), () => call('DELETE', path), 'disposition');
    if (e.scope === 'house') dashboard.house = null;
    else dashboard.mine = null;
    chooseActive('default');
    dashboard.editing = null;
    return true;
  } catch (err) {
    e.error = err.message;
    return false;
  } finally {
    if (dashboard.editing === e) e.busy = false;
  }
}

export { hub, home };
