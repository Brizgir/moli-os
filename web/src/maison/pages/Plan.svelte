<script>
  import { onMount } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import { hub, home, value, nameOf, deviceKind, isOn, toggle, num, note, roomOf, savePref, doorOpen, leak, motion, isGroup, isFixture, unpowered } from '../lib/home.svelte.js';
  import { asHuman, CANCELLED } from '../lib/auto.svelte.js';
  import Drawing from '../plan/Drawing.svelte';
  import House3D from '../plan/House3D.svelte';
  import { SKINS, SKIN_IDS, ITEMS } from '../plan/skins.js';
  import { spread, bbox } from '../plan/geometry.js';
  import * as E from '../plan/editing.js';
  import FilterChips from '../ui/FilterChips.svelte';
  import SpeakerCard from '../ui/SpeakerCard.svelte';
  import { keeps, loadFilter } from '../lib/filters.js';
  import { t, locale } from '../../lib/i18n.svelte.js';

  /** The house on its own plans: each device where it really is, alive. */
  let plan = $state({ floors: [] });
  let loaded = $state(false);
  let floorId = $state(null);
  /** 2D, on arriving: every floor side by side (until one is chosen). */
  let whole = $state(true);
  let ow = $state(0);
  let ih = $state(800);
  let editing = $state(false);
  let draft = $state(null);
  /** Edit: the device waiting for a tap on the plan. */
  let picking = $state(null);
  let search = $state('');
  /** View: the device whose card is open. */
  let opened = $state(null);
  let busy = $state(false);
  let newName = $state('');
  let surface = $state(null);
  let drawing = $state(null);
  let dragging = null;
  /** Edit, drawn floor: the layer arranged, what is chosen in it (and the
   *  room's chosen corner), the kinds to add, the magnet. */
  let layer = $state('items');
  let selection = $state(null);
  let vertex = $state(null);
  let addKind = $state('bed');
  let addOpeningKind = $state('door');
  let magnet = $state(true);
  /** Edit: the reference measured from (`{ type, i }`), and whether the
   *  next tap on the plan picks it. */
  let ruler = $state(null);
  let aiming = $state(false);
  /** Edit: the last changes, to undo and redo them. */
  let hist = $state(E.history());
  const LAYERS = ['items', 'rooms', 'walls', 'openings'];
  const LISTS = { item: 'items', room: 'rooms', wall: 'walls', opening: 'openings' };

  /** This viewer's skin (the house's default otherwise). */
  function readSkin() {
    try {
      return localStorage.getItem('plan-skin');
    } catch {
      return null;
    }
  }
  let mySkin = $state(readSkin());
  /** This viewer's sizes for the devices and the temperatures (small,
   *  medium, large), as the skin: a matter of screen. */
  const SIZES = { small: 0.75, medium: 1, large: 1.35 };
  function readSizes() {
    try {
      const v = JSON.parse(localStorage.getItem('plan-sizes') ?? '{}');
      return { devices: SIZES[v.devices] ? v.devices : 'medium', gauges: SIZES[v.gauges] ? v.gauges : 'medium' };
    } catch {
      return { devices: 'medium', gauges: 'medium' };
    }
  }
  let mySizes = $state(readSizes());
  const sizes = $derived({ devices: SIZES[mySizes.devices], gauges: SIZES[mySizes.gauges] });
  function chooseSize(what, v) {
    mySizes = { ...mySizes, [what]: v };
    savePref('plan-sizes', JSON.stringify(mySizes));
  }
  /** Which kinds of devices show (none chosen: all). Edit mode shows all. */
  let filter = $state(loadFilter('plan-filter'));
  // A Hue room is not a lamp, a fixture's bulbs and relay are the fixture:
  // none of them gets its own pin (D12).
  const inFixture = $derived(new Set(Object.values(hub.devices).filter(isFixture).flatMap((d) => d.members ?? [])));
  const pinnable = (d) => !isGroup(d) && !inFixture.has(d.id);
  const keep = (d) => pinnable(d) && (editing || keeps(filter, d));
  /** 2D or 3D, this viewer's choice. */
  let view = $state((() => {
    try {
      return localStorage.getItem('plan-view') === '3d' ? '3d' : '2d';
    } catch {
      return '2d';
    }
  })());

  function chooseView(v) {
    view = v;
    savePref('plan-view', v);
  }

  /** A tap on a device in 3D: lights switch, the rest says what it knows. */
  function tap3d(id) {
    const d = hub.devices[id];
    if (!d) return;
    if (['light', 'group', 'plug'].includes(deviceKind(d))) {
      if (!switchOf(d)) toggle(id);
      return;
    }
    if (deviceKind(d) === 'speaker') {
      opened = opened === id ? null : id;
      return;
    }
    const row = details(id)[0];
    const v = row ? (typeof row.v === 'number' ? num(row.v, 1) : row.v === true ? t('maison.plan.oui') : row.v === false ? t('maison.plan.non') : row.v) : null;
    note(row ? t('maison.plan.lecture', { name: nameOf(id), label: row.label, value: `${v}${row.unit ? ` ${row.unit}` : ''}` }) : nameOf(id));
  }

  const headers = { 'content-type': 'application/json', 'x-moli-origin': 'ui' };

  onMount(load);

  async function load() {
    try {
      const res = await fetch('/api/plan');
      plan = res.ok ? await res.json() : { floors: [] };
    } catch {
      plan = { floors: [] };
    }
    plan.floors ??= [];
    floorId = plan.floors.find((f) => f.id === floorId)?.id ?? plan.floors[0]?.id ?? null;
    loaded = true;
  }

  const shown = $derived(editing ? draft : plan);
  const floor = $derived(shown?.floors.find((f) => f.id === floorId) ?? shown?.floors[0] ?? null);
  const drawn = $derived((shown?.floors ?? []).some((f) => f.size));
  // The overview: the drawn floors at one scale, side by side, each at its
  // height on the first (the floor above lines up with the house below).
  const stack = $derived((shown?.floors ?? []).filter((f) => f.size));
  const overview = $derived(whole && view === '2d' && !editing && stack.length > 1);
  const GAP = 18;
  const scale = $derived.by(() => {
    const width = stack.reduce((t, f) => t + f.size[0], 0);
    const height = Math.max(...stack.map((f) => (f.offset?.[1] ?? 0) + f.size[1]));
    return Math.max(0.05, Math.min((ow - GAP * (stack.length - 1)) / width, Math.max(260, ih - 300) / height));
  });

  function pickFloor(id) {
    whole = false;
    floorId = id;
    selection = null;
    vertex = null;
    ruler = null;
    aiming = false;
  }
  const in3d = $derived(view === '3d' && !editing && drawn && floor?.size);
  const skinId = $derived(editing ? (draft?.skin ?? 'plan') : SKINS[mySkin] ? mySkin : (plan.skin ?? 'plan'));
  const skin = $derived(SKINS[skinId] ?? SKINS.plan);

  function chooseSkin(id) {
    if (editing) {
      change(() => (draft.skin = id));
      return;
    }
    mySkin = id;
    savePref('plan-skin', id);
  }
  const placed = $derived(new Set((shown?.floors ?? []).flatMap((f) => f.devices.map((d) => d.id))));
  const unplaced = $derived(
    Object.values(hub.devices)
      .filter((d) => !placed.has(d.id) && d.model !== 'Aide' && pinnable(d))
      .filter((d) => {
        const q = search.trim().toLowerCase();
        return !q || `${nameOf(d.id)} ${roomOf(d) ?? ''}`.toLowerCase().includes(q);
      })
      // By room, the devices in no room (weather, tariffs…) last.
      .sort(
        (a, b) =>
          Number(!roomOf(a)) - Number(!roomOf(b)) ||
          (roomOf(a) ?? '').localeCompare(roomOf(b) ?? '', locale()) ||
          nameOf(a.id).localeCompare(nameOf(b.id), locale()),
      ),
  );

  // ---- what a token shows -------------------------------------------------------

  const ICON = { light: 'light', group: 'light-group', plug: 'power', climate: 'air-conditioner', camera: 'cctv', speaker: 'speaker', tv: 'tv', cover: 'shutter' };
  const has = (d, key) => d.points.some((p) => p.key === key);

  function iconOf(d) {
    const kind = deviceKind(d);
    if (ICON[kind]) return ICON[kind];
    if (has(d, 'smoke')) return 'fire';
    if (has(d, 'water_leak') || has(d, 'watersensor_state')) return 'water';
    if (has(d, 'contact') || has(d, 'doorcontact_state')) return 'door';
    if (has(d, 'occupancy') || has(d, 'motion') || has(d, 'pir')) return 'motion';
    if (has(d, 'temperature') || has(d, 'temp_current')) return 'thermometer';
    return 'info';
  }

  /** A short reading next to the token (sensors). */
  function reading(d) {
    const t = value(d.id, 'temperature') ?? value(d.id, 'temp_current');
    if (typeof t === 'number' && deviceKind(d) !== 'climate' && !floor?.size) return `${num(t, 1)}°`;
    const p = value(d.id, 'power') ?? value(d.id, 'cur_power');
    if (typeof p === 'number' && p >= 1) return `${num(p)} W`;
    return null;
  }

  function alarming(d) {
    return value(d.id, 'smoke') === true || leak(d.id);
  }

  function opening(d) {
    return doorOpen(d.id) === true || motion(d.id);
  }

  function lit(d) {
    const kind = deviceKind(d);
    return ['light', 'group', 'plug', 'tv', 'speaker'].includes(kind) && isOn(d.id);
  }

  // ---- view ------------------------------------------------------------------------

  /** A lamp cut at the wall: only the switch brings it back (D12). */
  function switchOf(d) {
    if (!unpowered(d.id) || isFixture(d)) return false;
    note(t('maison.plan.coupee_mur', { name: nameOf(d.id) }));
    return true;
  }

  function tap(d) {
    if (editing) return;
    const kind = deviceKind(d);
    if (['light', 'group', 'plug'].includes(kind)) {
      if (!switchOf(d)) toggle(d.id);
      return;
    }
    if (kind === 'camera') location.hash = '#/cameras';
    else if (kind === 'tv') location.hash = '#/salon';
    else opened = opened === d.id ? null : d.id;
  }

  function details(id) {
    const d = hub.devices[id];
    if (!d) return [];
    return d.points
      .filter((p) => p.access?.read !== false)
      .map((p) => ({ label: p.label, v: value(id, p.key), unit: p.unit }))
      .filter((x) => x.v != null && x.v !== '')
      .slice(0, 6);
  }

  // ---- edit ------------------------------------------------------------------------

  function startEdit() {
    draft = structuredClone($state.snapshot(plan));
    editing = true;
    opened = null;
    hist = E.history();
    layer = 'items';
    selection = null;
    vertex = null;
    ruler = null;
    aiming = false;
  }

  function cancelEdit() {
    editing = false;
    draft = null;
    picking = null;
    selection = null;
    floorId = plan.floors.find((f) => f.id === floorId)?.id ?? plan.floors[0]?.id ?? null;
  }

  // ---- edit: undo, redo ------------------------------------------------------------

  const snapshot = () => JSON.stringify($state.snapshot(draft));

  /** A change starts (a drag, a field): what there was is kept. */
  function begin() {
    E.remember(hist, snapshot());
  }

  /** It ends: nothing changed (a tap) forgets it. */
  function end() {
    E.settle(hist, snapshot());
  }

  function change(fn) {
    begin();
    fn();
    end();
  }

  function restore(state) {
    if (state == null) return;
    draft = JSON.parse(state);
    if (!draft.floors.some((f) => f.id === floorId)) floorId = draft.floors[0]?.id ?? null;
    const f = draft.floors.find((x) => x.id === floorId);
    if (selection && !f?.[LISTS[selection.type]]?.[selection.i]) selection = null;
    if (ruler && !f?.[LISTS[ruler.type]]?.[ruler.i]) ruler = null;
    vertex = null;
  }

  const back = () => restore(E.undo(hist, snapshot()));
  const again = () => restore(E.redo(hist, snapshot()));

  function keys(e) {
    if (!editing || e.target.closest?.('input, select, textarea')) return;
    const key = e.key.toLowerCase();
    if ((e.ctrlKey || e.metaKey) && key === 'z') {
      e.preventDefault();
      if (e.shiftKey) again();
      else back();
    } else if ((e.ctrlKey || e.metaKey) && key === 'y') {
      e.preventDefault();
      again();
    } else if ((e.key === 'Delete' || e.key === 'Backspace') && chosen) {
      e.preventDefault();
      removeChosen();
    }
  }

  function slug(name) {
    const base = name
      .normalize('NFD')
      .replace(/[̀-ͯ]/g, '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 24) || 'etage';
    let id = base;
    for (let n = 2; draft.floors.some((f) => f.id === id); n++) id = `${base}-${n}`;
    return id;
  }

  async function uploadImage(file) {
    const res = await fetch('/api/plan/images', {
      method: 'POST',
      headers: { 'content-type': file.type || 'application/octet-stream', 'x-moli-origin': 'ui' },
      body: file,
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) {
      const err = new Error(body.error ?? res.statusText);
      err.status = res.status;
      throw err;
    }
    return body.image;
  }

  async function addFloor(event) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = '';
    const name = newName.trim() || t('maison.plan.etage_defaut', { n: draft.floors.length + 1 });
    try {
      busy = true;
      const image = file ? await asHuman(t('maison.plan.ajouter_plan', { name }), () => uploadImage(file)) : null;
      const id = slug(name);
      change(() => draft.floors.push({ id, name, ...(image ? { image } : {}), devices: [] }));
      floorId = id;
      newName = '';
    } catch (err) {
      if (err.message !== CANCELLED) note(err.message, 'error');
    } finally {
      busy = false;
    }
  }

  /** A floor drawn on a grid, until its plan's image comes. */
  function addBlankFloor() {
    const name = newName.trim() || t('maison.plan.etage_defaut', { n: draft.floors.length + 1 });
    const id = slug(name);
    change(() => draft.floors.push({ id, name, devices: [] }));
    floorId = id;
    newName = '';
  }

  async function replaceImage(event) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = '';
    if (!file || !floor) return;
    try {
      busy = true;
      const image = await asHuman(t('maison.plan.changer_plan', { name: floor.name }), () => uploadImage(file));
      change(() => (floor.image = image));
    } catch (err) {
      if (err.message !== CANCELLED) note(err.message, 'error');
    } finally {
      busy = false;
    }
  }

  function removeFloor() {
    if (!floor || !confirm(t('maison.plan.retirer_confirme', { name: floor.name }))) return;
    change(() => (draft.floors = draft.floors.filter((f) => f.id !== floor.id)));
    floorId = draft.floors[0]?.id ?? null;
  }

  /** Where a pointer is on the plan: cm on a drawn floor, 0–1 on an image. */
  function at(event) {
    if (floor?.size && drawing) {
      const p = drawing.toWorld(event.clientX, event.clientY);
      const [w, h] = floor.size;
      return { x: Math.round(Math.min(w, Math.max(0, p.x))), y: Math.round(Math.min(h, Math.max(0, p.y))) };
    }
    const box = surface.getBoundingClientRect();
    const clamp = (v) => Math.min(1, Math.max(0, Math.round(v * 10000) / 10000));
    return { x: clamp((event.clientX - box.left) / box.width), y: clamp((event.clientY - box.top) / box.height) };
  }

  function placeHere(event) {
    if (!editing || !picking || !floor) return;
    change(() => floor.devices.push({ id: picking, ...at(event) }));
    picking = null;
  }

  function grab(event, spot) {
    if (!editing) return;
    event.preventDefault();
    // Not a pan of the drawing: this device moves.
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    begin();
    dragging = spot;
  }

  function drag(event) {
    if (!dragging) return;
    Object.assign(dragging, at(event));
  }

  function drop() {
    if (dragging) end();
    dragging = null;
  }

  function unplace(spot) {
    change(() => (floor.devices = floor.devices.filter((d) => d !== spot)));
  }

  function placeAt(x, y) {
    if (!editing || !picking || !floor) return;
    change(() => floor.devices.push({ id: picking, x, y }));
    picking = null;
  }

  // ---- drawn floor: fixtures and placing by room ---------------------------------

  /** What is chosen on the floor (a fixture, a room, a wall, an opening). */
  const chosen = $derived(editing && selection && floor ? (floor[LISTS[selection.type]]?.[selection.i] ?? null) : null);
  /** A chosen wall as the plain box it covers (a quarter turn undone). */
  const wallBox = $derived(selection?.type === 'wall' && chosen ? E.unturned(chosen) : null);
  /** The wall the chosen opening sits in (its plain box). */
  const openingWall = $derived(selection?.type === 'opening' && chosen ? E.wallOf(chosen, floor.walls ?? []) : null);
  const openingWidth = $derived(selection?.type === 'opening' && chosen ? Math.round(E.widthOf(chosen, floor.walls ?? [])) : null);
  /** The Moli rooms a drawn room can be (home.json), and the one it says. */
  const moliRooms = $derived.by(() => {
    const names = (home.config?.rooms ?? []).map((r) => r.name);
    const own = selection?.type === 'room' ? chosen?.room : null;
    return own && !names.includes(own) ? [...names, own] : names;
  });

  // ---- measuring: a reference, the gap from it to what is chosen ----------------------

  /** The box a thing covers (a room: around its corners). */
  function boxOf(sel) {
    const s = sel && floor ? floor[LISTS[sel.type]]?.[sel.i] : null;
    if (!s) return null;
    return sel.type === 'room' ? bbox(s.poly) : E.footprint(s);
  }
  const refBox = $derived(editing && ruler ? boxOf(ruler) : null);
  const sameAsRef = $derived(!!(ruler && selection && ruler.type === selection.type && ruler.i === selection.i));
  const chosenBox = $derived(refBox && chosen && !sameAsRef ? boxOf(selection) : null);
  const cotes = $derived(refBox && chosenBox ? E.gaps(refBox, chosenBox) : null);
  /** What the reference is called in the sheet. */
  const refName = $derived.by(() => {
    const s = editing && ruler && floor ? floor[LISTS[ruler.type]]?.[ruler.i] : null;
    if (!s) return '';
    // « du mur », « de la porte », « de « Salon » » : each with its own words.
    if (ruler.type === 'room') return t('maison.plan.repere.piece', { name: s.name });
    if (ruler.type === 'item') return t('maison.plan.repere.meuble', { name: ITEMS[s.kind]?.name ?? s.kind });
    if (ruler.type === 'opening') return t(`maison.plan.repere.${s.kind}`);
    return t('maison.plan.repere.mur');
  });

  /** The tap on the plan picks the reference (or nothing: stop aiming). */
  function pickRef(sel) {
    aiming = false;
    if (sel) ruler = sel;
  }

  /** What is chosen, moved by (dx, dy) cm (an opening stays in a wall). */
  function shiftChosen(dx, dy) {
    const { type, i } = selection;
    if (type === 'wall') return setWall((b) => E.clampBox({ ...b, x: b.x + dx, y: b.y + dy }, floor.size));
    change(() => {
      if (type === 'item') Object.assign(chosen, E.clampBox({ ...$state.snapshot(chosen), x: chosen.x + dx, y: chosen.y + dy }, floor.size));
      else if (type === 'room') chosen.poly = chosen.poly.map(([x, y]) => E.clampPoint([x + dx, y + dy], floor.size));
      else {
        const o = $state.snapshot(chosen);
        const [cx, cy] = E.centerOf(o);
        floor.openings[i] = E.placeOpening($state.snapshot(floor.walls ?? []), o, E.widthOf(o, floor.walls ?? []), cx + dx, cy + dy, false);
      }
    });
  }

  /** The gap typed: what is chosen moves to be that far from the reference. */
  function gapTo(axis, event) {
    const v = cm(event, -10000, 10000);
    if (v == null || !cotes) return;
    const next = E.withGap(refBox, chosenBox, axis, v);
    shiftChosen(next.x - chosenBox.x, next.y - chosenBox.y);
  }

  function chooseLayer(id) {
    layer = id;
    selection = null;
    vertex = null;
  }

  /** The middle of what is on screen, inside the floor (`m` cm from its edges). */
  function middle(m = 0) {
    const c = drawing.center();
    const [w, h] = floor.size;
    return { x: Math.min(w - m, Math.max(m, c.x)), y: Math.min(h - m, Math.max(m, c.y)) };
  }

  function addItem() {
    const kind = ITEMS[addKind];
    const c = middle();
    change(() => {
      const item = { kind: addKind, x: Math.round(c.x - kind.w / 2), y: Math.round(c.y - kind.h / 2), w: kind.w, h: kind.h };
      floor.items = [...(floor.items ?? []), E.clampBox(item, floor.size)];
      selection = { type: 'item', i: floor.items.length - 1 };
    });
  }

  function turnItem() {
    if (chosen) change(() => (chosen.r = ((chosen.r ?? 0) + 90) % 360));
  }

  /** The chosen thing goes (a wall takes its openings with it). */
  function removeChosen() {
    if (!chosen) return;
    const { type, i } = selection;
    change(() => {
      if (type === 'wall') {
        const on = new Set(E.openingsOn(floor.walls[i], floor.openings ?? []));
        if (on.size) floor.openings = floor.openings.filter((_, k) => !on.has(k));
      }
      floor[LISTS[type]] = floor[LISTS[type]].filter((_, k) => k !== i);
      // The list shifted: a reference in it is not what it was.
      if (ruler?.type === type || (type === 'wall' && ruler?.type === 'opening')) ruler = null;
      selection = null;
      vertex = null;
    });
  }

  // ---- drawn floor: rooms -----------------------------------------------------------

  function addRoom() {
    const c = middle(150);
    change(() => {
      const taken = draft.floors.flatMap((f) => (f.rooms ?? []).map((r) => r.id));
      const room = { id: E.freshId(taken), name: t('maison.plan.nouvelle_piece'), kind: 'other', poly: E.rectRoom(c.x, c.y) };
      floor.rooms = [...(floor.rooms ?? []), room];
      selection = { type: 'room', i: floor.rooms.length - 1 };
      vertex = null;
    });
  }

  function renameRoom(event) {
    const name = event.currentTarget.value.trim().slice(0, 60);
    if (name && name !== chosen.name) change(() => (chosen.name = name));
    else event.currentTarget.value = chosen.name;
  }

  function setRoomKind(kind) {
    change(() => (chosen.kind = kind));
  }

  function setMoliRoom(name) {
    change(() => {
      if (name) chosen.room = name;
      else delete chosen.room;
    });
  }

  /** Where the chosen room's temperature shows, typed in cm (empty: its foot). */
  function gaugeField(k, event) {
    const v = cm(event, -E.MARGIN, 10000);
    change(() => {
      if (v == null) {
        delete chosen.gauge;
        return;
      }
      const at = chosen.gauge ? [...chosen.gauge] : [bbox(chosen.poly).x + bbox(chosen.poly).w / 2, bbox(chosen.poly).y + bbox(chosen.poly).h];
      at[k] = v;
      chosen.gauge = E.clampPoint(at, floor.size);
    });
  }

  function removeCorner() {
    if (vertex == null || chosen.poly.length <= 3) return;
    change(() => {
      chosen.poly = E.removeVertex($state.snapshot(chosen.poly), vertex);
      vertex = null;
    });
  }

  // ---- drawn floor: walls and openings ----------------------------------------------

  function addWall(along) {
    const c = middle(160);
    change(() => {
      floor.walls = [...(floor.walls ?? []), E.clampBox(E.newWall(c.x, c.y, along), floor.size)];
      selection = { type: 'wall', i: floor.walls.length - 1 };
    });
  }

  /** The chosen wall changed by `fn` (on its plain box); its openings
   *  follow (as far from its start as before). */
  function setWall(fn) {
    const i = selection.i;
    change(() => {
      const before = E.unturned($state.snapshot(floor.walls[i]));
      const after = fn(before);
      floor.walls[i] = after;
      if (!E.straight(after) || after.r != null) return;
      const [dx, dy] = [after.x - before.x, after.y - before.y];
      for (const k of E.openingsOn(before, floor.openings ?? [])) floor.openings[k] = E.carryOpening($state.snapshot(floor.openings[k]), before, after, dx, dy);
    });
  }

  /** Where the chosen wall starts (its top left corner), cm from the floor's. */
  function wallAt(axis, event) {
    const v = cm(event, -E.MARGIN, 10000);
    if (v != null) setWall((b) => E.clampBox({ ...b, [axis]: v }, floor.size));
  }

  /** The chosen fixture, typed in cm: where it is, how big. */
  function itemField(key, event) {
    const v = key === 'x' || key === 'y' ? cm(event, -E.MARGIN, 10000) : cm(event, 10, 5000);
    if (v == null) return;
    change(() => Object.assign(chosen, E.clampBox({ ...$state.snapshot(chosen), [key]: v }, floor.size)));
  }

  /** The chosen corner of the chosen room, typed in cm. */
  function cornerAt(k, event) {
    const v = cm(event, -E.MARGIN, 10000);
    if (v == null || vertex == null) return;
    change(() => {
      const p = [...chosen.poly[vertex]];
      p[k] = v;
      chosen.poly[vertex] = E.clampPoint(p, floor.size);
    });
  }

  /** A number from a field, within bounds (the field shows it); `null` when empty. */
  function cm(event, min, max) {
    const field = event.currentTarget;
    const v = Number.parseFloat(String(field.value).replace(',', '.'));
    if (!Number.isFinite(v)) return null;
    const out = Math.min(max, Math.max(min, Math.round(v)));
    field.value = String(out);
    return out;
  }

  function wallThickness(event) {
    const v = cm(event, 1, 200);
    if (v) setWall((b) => E.setThickness(b, v));
  }

  function wallLength(event) {
    const v = cm(event, E.MIN_WALL, 10000);
    if (v) setWall((b) => E.clampBox(E.setLength(b, v), floor.size));
  }

  /** Empty: as high as the floor's walls; else a low wall, a railing. */
  function wallHeight(event) {
    const v = cm(event, 1, 1000);
    setWall(({ height, ...rest }) => (v ? { ...rest, height: v } : rest));
  }

  function addOpening() {
    const c = middle();
    change(() => {
      const width = E.OPENING_WIDTH[addOpeningKind];
      const o = E.placeOpening($state.snapshot(floor.walls ?? []), { kind: addOpeningKind, x: 0, y: 0, w: width, h: 20 }, width, c.x, c.y);
      floor.openings = [...(floor.openings ?? []), E.clampBox(o, floor.size)];
      selection = { type: 'opening', i: floor.openings.length - 1 };
    });
  }

  /** A door or a window in the middle of the chosen wall, then chosen
   *  (its distance from the wall's start is typed next). */
  function addOpeningInWall() {
    const wall = wallBox;
    change(() => {
      const width = Math.min(E.OPENING_WIDTH[addOpeningKind], E.lengthOf(wall));
      const o = { kind: addOpeningKind, ...E.openingAt(wall, width, (E.lengthOf(wall) - width) / 2) };
      floor.openings = [...(floor.openings ?? []), o];
      layer = 'openings';
      selection = { type: 'opening', i: floor.openings.length - 1 };
      vertex = null;
    });
  }

  function setOpeningKind(kind) {
    change(() => (chosen.kind = kind));
  }

  /** The chosen opening, so many cm from the start of its wall. */
  function openingOffsetTo(event) {
    const wall = openingWall;
    const v = cm(event, 0, 10000);
    if (v == null || !wall) return;
    const i = selection.i;
    change(() => (floor.openings[i] = { ...$state.snapshot(floor.openings[i]), ...E.openingAt(wall, openingWidth, v) }));
  }

  function openingWidthTo(event) {
    const v = cm(event, E.MIN_OPENING, 2000);
    if (!v) return;
    const i = selection.i;
    const wall = openingWall;
    change(() => {
      const o = $state.snapshot(floor.openings[i]);
      // In a wall: as far from its start as typed; else around its middle.
      floor.openings[i] = wall ? { ...o, ...E.openingAt(wall, v, E.offsetOf(o, wall)) } : E.resizeOpening($state.snapshot(floor.walls ?? []), o, v);
    });
  }

  /** Every device not on the plan yet goes into its room, if this floor has it. */
  function placeByRoom() {
    let placedNow = 0;
    begin();
    for (const r of floor.rooms ?? []) {
      if (!r.room) continue;
      const mine = unplaced.filter((d) => roomOf(d) === r.room);
      spread(r.poly, mine.length).forEach(([x, y], i) => {
        floor.devices.push({ id: mine[i].id, x, y });
        placedNow++;
      });
    }
    end();
    note(placedNow ? t('maison.plan.poses', { count: placedNow }) : t('maison.plan.rien_a_poser'));
  }

  async function save() {
    busy = true;
    try {
      const saved = await asHuman(t('maison.plan.enregistrer_plan'), async () => {
        const res = await fetch('/api/plan', { method: 'PUT', headers, body: JSON.stringify($state.snapshot(draft)) });
        const body = await res.json().catch(() => ({}));
        if (!res.ok) {
          const err = new Error(body.error ?? res.statusText);
          err.status = res.status;
          throw err;
        }
        return body;
      });
      plan = saved;
      editing = false;
      draft = null;
      picking = null;
      selection = null;
      note(t('maison.plan.enregistre'));
    } catch (err) {
      if (err.message !== CANCELLED) note(err.message, 'error');
    } finally {
      busy = false;
    }
  }
</script>

{#snippet spotBody(spot, d)}
  <button
    class="token"
    class:lit={lit(d)}
    class:alarm={alarming(d)}
    class:open={opening(d)}
    class:off={d.online === false}
    onclick={(e) => { e.stopPropagation(); tap(d); }}
    onpointerdown={(e) => grab(e, spot)}
    aria-label={lit(d) ? t('maison.plan.jeton_allume', { name: nameOf(d.id) }) : nameOf(d.id)}
    title={nameOf(d.id)}
  >
    <Icon name={iconOf(d)} size={18} />
  </button>
  {#if reading(d)}<span class="reading">{reading(d)}</span>{/if}
  <span class="name">{nameOf(d.id)}</span>
  {#if editing}
    <button class="unplace" onclick={(e) => { e.stopPropagation(); unplace(spot); }} aria-label={t('maison.plan.retirer_du_plan', { name: nameOf(d.id) })}><Icon name="close" size={12} /></button>
  {/if}
  {#if opened === d.id && deviceKind(d) !== 'speaker'}
    <div class="card" role="dialog" aria-label={nameOf(d.id)}>
      <b>{nameOf(d.id)}</b>
      {#if roomOf(d)}<small class="muted">{roomOf(d)}</small>{/if}
      {#each details(d.id) as row (row.label)}
        <p><span>{row.label}</span><b>{typeof row.v === 'number' ? num(row.v, 1) : row.v === true ? t('maison.plan.oui') : row.v === false ? t('maison.plan.non') : row.v}{row.unit ? ` ${row.unit}` : ''}</b></p>
      {/each}
    </div>
  {/if}
{/snippet}

<svelte:window bind:innerHeight={ih} onkeydown={keys} />

<div class="plan" data-unsaved={editing || undefined}>
  <header>
    <div>
      <h1 class="page-title">{t('maison.plan.titre')}</h1>
      <p class="page-sub">{editing ? t('maison.plan.sous_titre_edition') : t('maison.plan.sous_titre')}</p>
    </div>
    <div class="actions">
      {#if editing}
        <button class="ghost" onclick={cancelEdit} disabled={busy}>{t('commun.annuler')}</button>
        <button class="primary" onclick={save} disabled={busy}><Icon name="lock" size={16} />{t('maison.plan.enregistrer')}</button>
      {:else if loaded && plan.floors.length}
        <button class="ghost" onclick={startEdit}><Icon name="tune" size={16} />{t('maison.plan.modifier')}</button>
      {/if}
    </div>
  </header>

  {#if shown?.floors.length}
    <div class="floors" role="tablist">
      {#if stack.length > 1 && view === '2d' && !editing}
        <button role="tab" aria-selected={overview} class:active={overview} onclick={() => (whole = true)}>{t('maison.plan.toute_maison')}</button>
      {/if}
      {#each shown.floors as f (f.id)}
        <button role="tab" aria-selected={!overview && floor?.id === f.id} class:active={!overview && floor?.id === f.id} onclick={() => pickFloor(f.id)}>{f.name}</button>
      {/each}
    </div>
  {/if}

  {#if floor && !editing}
    <FilterChips devices={(overview ? stack : [floor]).flatMap((f) => f.devices).map((s) => hub.devices[s.id]).filter(Boolean)} bind:filter key="plan-filter" />
  {/if}

  {#if floor?.size}
    <div class="skins" role="radiogroup" aria-label={t('maison.plan.habillage_plan')}>
      {#if !editing}
        <span class="views">
          <button class:active={view === '2d'} onclick={() => chooseView('2d')} aria-pressed={view === '2d'}>2D</button>
          <button class:active={view === '3d'} onclick={() => chooseView('3d')} aria-pressed={view === '3d'}>3D</button>
        </span>
      {/if}
      <span class="muted small">{editing ? t('maison.plan.habillage_maison') : t('maison.plan.habillage')}</span>
      {#each SKIN_IDS as id (id)}
        <button role="radio" aria-checked={skinId === id} class:active={skinId === id} onclick={() => chooseSkin(id)}>
          <i style="background:{SKINS[id].bg}; border-color:{SKINS[id].wall}"></i>{SKINS[id].name}
        </button>
      {/each}
      {#if !editing && !in3d}
        {#each ['devices', 'gauges'] as what (what)}
          <label class="size">
            <span class="muted small">{t(`maison.plan.taille.${what}`)}</span>
            <select value={mySizes[what]} onchange={(e) => chooseSize(what, e.currentTarget.value)}>
              {#each Object.keys(SIZES) as v (v)}<option value={v}>{t(`maison.plan.taille.${v}`)}</option>{/each}
            </select>
          </label>
        {/each}
      {/if}
    </div>
  {/if}

  {#if editing && floor?.size}
    <div class="tools">
      <span class="seg" role="group" aria-label={t('maison.plan.calques')}>
        {#each LAYERS as id (id)}
          <button class:active={layer === id} aria-pressed={layer === id} onclick={() => chooseLayer(id)}>{t(`maison.plan.calque.${id}`)}</button>
        {/each}
      </span>
      <span class="row">
        <button class="chip" onclick={back} disabled={!hist.past.length} title={t('maison.plan.defaire_aide')} aria-label={t('maison.plan.defaire')}><Icon name="undo" size={18} /><span class="word">{t('maison.plan.defaire')}</span></button>
        <button class="chip" onclick={again} disabled={!hist.future.length} title={t('maison.plan.refaire_aide')} aria-label={t('maison.plan.refaire')}><Icon name="redo" size={18} /><span class="word">{t('maison.plan.refaire')}</span></button>
        <label class="chip" title={t('maison.plan.magnetisme_aide')}><input type="checkbox" bind:checked={magnet} /><Icon name="magnet" size={16} /><span class="word">{t('maison.plan.magnetisme')}</span></label>
      </span>
    </div>
    <div class="tools">
      {#if layer === 'items'}
        <label class="pick">
          <span class="muted small">{t('maison.plan.ajouter')}</span>
          <select bind:value={addKind} aria-label={t('maison.plan.element_a_ajouter')}>
            {#each Object.entries(ITEMS) as [id, it] (id)}<option value={id}>{it.name}</option>{/each}
          </select>
          <button class="link" onclick={addItem}><Icon name="plus" size={15} />{t('maison.plan.ajouter')}</button>
        </label>
        <button class="link" onclick={placeByRoom}>{t('maison.plan.poser_par_piece')}</button>
      {:else if layer === 'rooms'}
        <button class="chip" onclick={addRoom}><Icon name="plus" size={16} />{t('maison.plan.ajouter_piece')}</button>
        <span class="muted small">{t('maison.plan.aide.rooms')}</span>
      {:else if layer === 'walls'}
        <button class="chip" onclick={() => addWall('x')}><Icon name="plus" size={16} />{t('maison.plan.mur_horizontal')}</button>
        <button class="chip" onclick={() => addWall('y')}><Icon name="plus" size={16} />{t('maison.plan.mur_vertical')}</button>
        <span class="muted small">{t('maison.plan.aide.walls')}</span>
      {:else}
        <label class="pick">
          <select bind:value={addOpeningKind} aria-label={t('maison.plan.ouverture_a_ajouter')}>
            {#each E.OPENING_KINDS as k (k)}<option value={k}>{t(`maison.plan.ouverture.${k}`)}</option>{/each}
          </select>
          <button class="chip" onclick={addOpening}><Icon name="plus" size={16} />{t('maison.plan.ajouter')}</button>
        </label>
        <span class="muted small">{t('maison.plan.aide.openings')}</span>
      {/if}
    </div>
  {/if}

  {#if !loaded}
    <p class="muted">…</p>
  {:else if !floor}
    <section class="empty">
      <Icon name="home-roof" size={40} />
      <h2>{t('maison.plan.vide_titre')}</h2>
      <p>{t('maison.plan.vide_texte')}</p>
      {#if !editing}<button class="primary" onclick={startEdit}><Icon name="plus" size={16} />{t('maison.plan.creer')}</button>{/if}
    </section>
  {:else}
    <div class="board" class:editing>
      {#if overview}
        <div class="whole" bind:clientWidth={ow} style="--gap:{GAP}px">
          {#each stack as f (f.id)}
            <div class="storey" style="width:{f.size[0] * scale}px">
              <button class="storey-name" onclick={() => pickFloor(f.id)}>{f.name}<Icon name="arrow-top-right" size={14} /></button>
              <div style="margin-top:{(f.offset?.[1] ?? 0) * scale}px; height:{f.size[1] * scale}px">
                <Drawing floor={f} {skin} {keep} {sizes} fill ontap={() => (opened = null)} token={spotBody} />
              </div>
            </div>
          {/each}
        </div>
      {:else if in3d}
        <House3D plan={shown} floorId={floor.id} {skin} {keep} ontap={tap3d} />
      {:else if floor.size}
        <div class="drawn" onpointermove={drag} onpointerup={drop} onpointercancel={drop} role="presentation">
          <Drawing bind:this={drawing} {floor} {skin} {editing} {picking} {keep} {layer} {magnet} {aiming} {ruler} {sizes} onpick={pickRef} bind:selection bind:vertex onbegin={begin} onend={end} onplace={placeAt} ontap={() => (opened = null)} token={spotBody} />
          <!-- What is chosen, under the plan: choosing does not move the plan. -->
          {#if chosen}
            <section class="sheet" aria-label={t('maison.plan.fiche')}>
              {#if selection.type === 'item'}
                <label><span>{t('maison.plan.pos_x')}</span><input type="number" inputmode="numeric" value={Math.round(chosen.x)} onchange={(e) => itemField('x', e)} /></label>
                <label><span>{t('maison.plan.pos_y')}</span><input type="number" inputmode="numeric" value={Math.round(chosen.y)} onchange={(e) => itemField('y', e)} /></label>
                <label><span>{t('maison.plan.largeur')}</span><input type="number" inputmode="numeric" min="10" value={Math.round(chosen.w)} onchange={(e) => itemField('w', e)} /></label>
                <label><span>{t('maison.plan.profondeur')}</span><input type="number" inputmode="numeric" min="10" value={Math.round(chosen.h)} onchange={(e) => itemField('h', e)} /></label>
                <p class="muted small">{t('maison.plan.aide.origine')}</p>
                <div class="row">
                  <button class="chip" onclick={turnItem}>{t('maison.plan.tourner')}</button>
                  <button class="chip danger" onclick={removeChosen}><Icon name="trash" size={16} />{t('maison.plan.retirer_element')}</button>
                </div>
              {:else if selection.type === 'room'}
                <label><span>{t('maison.plan.piece_nom')}</span><input value={chosen.name} maxlength="60" onchange={renameRoom} /></label>
                <label><span>{t('maison.plan.piece_type')}</span>
                  <select value={chosen.kind} onchange={(e) => setRoomKind(e.currentTarget.value)}>
                    {#each E.ROOM_KINDS as k (k)}<option value={k}>{t(`maison.plan.type_piece.${k}`)}</option>{/each}
                  </select>
                </label>
                <label><span>{t('maison.plan.piece_moli')}</span>
                  <select value={chosen.room ?? ''} onchange={(e) => setMoliRoom(e.currentTarget.value)}>
                    <option value="">{t('maison.plan.aucune')}</option>
                    {#each moliRooms as n (n)}<option value={n}>{n}</option>{/each}
                  </select>
                </label>
                {#if vertex != null && chosen.poly[vertex]}
                  <label><span>{t('maison.plan.coin_x', { n: vertex + 1 })}</span><input type="number" inputmode="numeric" value={Math.round(chosen.poly[vertex][0])} onchange={(e) => cornerAt(0, e)} /></label>
                  <label><span>{t('maison.plan.coin_y', { n: vertex + 1 })}</span><input type="number" inputmode="numeric" value={Math.round(chosen.poly[vertex][1])} onchange={(e) => cornerAt(1, e)} /></label>
                {/if}
                <p class="muted small">{t('maison.plan.aide.coins', { n: chosen.poly.length })} {t('maison.plan.aide.origine')}</p>
                {#if chosen.kind !== 'void' && chosen.kind !== 'roof'}
                  <label><span>{t('maison.plan.temperature_x')}</span><input type="number" inputmode="numeric" value={chosen.gauge ? Math.round(chosen.gauge[0]) : ''} placeholder={t('maison.plan.auto')} onchange={(e) => gaugeField(0, e)} /></label>
                  <label><span>{t('maison.plan.temperature_y')}</span><input type="number" inputmode="numeric" value={chosen.gauge ? Math.round(chosen.gauge[1]) : ''} placeholder={t('maison.plan.auto')} onchange={(e) => gaugeField(1, e)} /></label>
                  {#if chosen.gauge}<button class="chip" onclick={() => change(() => delete chosen.gauge)}>{t('maison.plan.temperature_auto')}</button>{/if}
                  <p class="muted small">{t('maison.plan.aide.temperature')}</p>
                {/if}
                <div class="row">
                  <button class="chip" onclick={removeCorner} disabled={vertex == null || chosen.poly.length <= 3}>{t('maison.plan.retirer_coin')}</button>
                  <button class="chip danger" onclick={removeChosen}><Icon name="trash" size={16} />{t('maison.plan.retirer_piece')}</button>
                </div>
              {:else if selection.type === 'wall'}
                <label><span>{t('maison.plan.pos_x')}</span><input type="number" inputmode="numeric" value={Math.round(wallBox.x)} onchange={(e) => wallAt('x', e)} /></label>
                <label><span>{t('maison.plan.pos_y')}</span><input type="number" inputmode="numeric" value={Math.round(wallBox.y)} onchange={(e) => wallAt('y', e)} /></label>
                <label><span>{t('maison.plan.epaisseur')}</span><input type="number" inputmode="numeric" min="1" max="200" value={E.thicknessOf(wallBox)} onchange={wallThickness} /></label>
                <label><span>{t('maison.plan.longueur')}</span><input type="number" inputmode="numeric" min={E.MIN_WALL} max="10000" value={E.lengthOf(wallBox)} onchange={wallLength} /></label>
                <label><span>{t('maison.plan.hauteur')}</span><input type="number" inputmode="numeric" min="1" max="1000" value={chosen.height ?? ''} placeholder={t('maison.plan.pleine_hauteur')} onchange={wallHeight} /></label>
                <p class="muted small">{t('maison.plan.aide.origine')}</p>
                {#if !E.straight(chosen)}<p class="muted small">{t('maison.plan.mur_biais')}</p>{/if}
                <div class="row">
                  {#if E.straight(chosen)}
                    <select bind:value={addOpeningKind} aria-label={t('maison.plan.ouverture_a_ajouter')}>
                      {#each E.OPENING_KINDS as k (k)}<option value={k}>{t(`maison.plan.ouverture.${k}`)}</option>{/each}
                    </select>
                    <button class="chip" onclick={addOpeningInWall}><Icon name="plus" size={16} />{t('maison.plan.ajouter_dans_mur')}</button>
                  {/if}
                  <button class="chip danger" onclick={removeChosen}><Icon name="trash" size={16} />{t('maison.plan.retirer_mur')}</button>
                </div>
              {:else}
                <label><span>{t('maison.plan.piece_type')}</span>
                  <select value={chosen.kind} onchange={(e) => setOpeningKind(e.currentTarget.value)}>
                    {#each E.OPENING_KINDS as k (k)}<option value={k}>{t(`maison.plan.ouverture.${k}`)}</option>{/each}
                  </select>
                </label>
                <label><span>{t('maison.plan.largeur')}</span><input type="number" inputmode="numeric" min={E.MIN_OPENING} max="2000" value={openingWidth} onchange={openingWidthTo} /></label>
                {#if openingWall}
                  <label><span>{t('maison.plan.depuis_debut')}</span><input type="number" inputmode="numeric" min="0" value={Math.round(E.offsetOf(chosen, openingWall))} onchange={openingOffsetTo} /></label>
                  <p class="muted small">{t(E.horizontal(openingWall) ? 'maison.plan.aide.debut_gauche' : 'maison.plan.aide.debut_haut', { len: Math.round(E.lengthOf(openingWall)) })}</p>
                {/if}
                <div class="row">
                  <button class="chip danger" onclick={removeChosen}><Icon name="trash" size={16} />{t('maison.plan.retirer_ouverture')}</button>
                </div>
              {/if}
              <!-- measuring: from a reference picked on the plan to what is chosen -->
              <div class="measure">
                {#if cotes}
                  <label><span>{t(cotes.x.after ? 'maison.plan.cote.droite' : 'maison.plan.cote.gauche', { name: refName })}</span><input type="number" inputmode="numeric" value={Math.round(cotes.x.gap)} onchange={(e) => gapTo('x', e)} /></label>
                  <label><span>{t(cotes.y.after ? 'maison.plan.cote.dessous' : 'maison.plan.cote.dessus', { name: refName })}</span><input type="number" inputmode="numeric" value={Math.round(cotes.y.gap)} onchange={(e) => gapTo('y', e)} /></label>
                  <p class="muted small">{t('maison.plan.cote.aide')}</p>
                {:else if ruler && sameAsRef}
                  <p class="muted small">{t('maison.plan.cote.meme', { name: refName })}</p>
                {/if}
                <div class="row">
                  <button class="chip" class:active={aiming} aria-pressed={aiming} onclick={() => (aiming = !aiming)}><Icon name="ruler" size={16} />{t(ruler ? 'maison.plan.mesurer_autre' : 'maison.plan.mesurer')}</button>
                  {#if ruler}<button class="chip" onclick={() => ((ruler = null), (aiming = false))}>{t('maison.plan.mesure_arreter')}</button>{/if}
                  {#if aiming}<span class="muted small">{t('maison.plan.mesurer_aide')}</span>{/if}
                </div>
              </div>
            </section>
          {/if}
        </div>
      {:else}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="surface" class:picking={!!picking} class:blank={!floor.image} bind:this={surface} onclick={placeHere} onpointermove={drag} onpointerup={drop} onpointercancel={drop}>
        {#if floor.image}
          <img src="/api/plan/images/{floor.image}" alt={t('maison.plan.nom_etage', { name: floor.name })} draggable="false" />
        {/if}
        {#each floor.devices as spot (spot.id)}
          {@const d = hub.devices[spot.id]}
          {#if d && keep(d)}
            <div class="spot" style="left:{spot.x * 100}%; top:{spot.y * 100}%">
              {@render spotBody(spot, d)}
            </div>
          {/if}
        {/each}
      </div>
      {/if}

      <!-- A speaker's controls open under the plan (a tap on the plan closes them). -->
      {#if !editing && opened && hub.devices[opened] && deviceKind(hub.devices[opened]) === 'speaker'}
        <SpeakerCard id={opened} />
      {/if}

      {#if editing}
        <aside class="tray">
          <input placeholder={t('maison.plan.chercher')} bind:value={search} aria-label={t('maison.plan.chercher_aria')} />
          <p class="muted small">{t('maison.plan.a_placer', { n: unplaced.length })}{picking ? ` · ${t('maison.plan.toucher_pour_poser', { name: nameOf(picking) })}` : ''}</p>
          <ul>
            {#each unplaced as d (d.id)}
              <li>
                <button class:chosen={picking === d.id} onclick={() => (picking = picking === d.id ? null : d.id)}>
                  <Icon name={iconOf(d)} size={16} /><span>{nameOf(d.id)}</span>{#if roomOf(d)}<small>{roomOf(d)}</small>{/if}
                </button>
              </li>
            {/each}
          </ul>
        </aside>
      {/if}
    </div>
  {/if}

  <!-- Floors come and go rarely: under the plan. -->
  {#if editing}
    <div class="tools">
      <label class="add">
        <input placeholder={t('maison.plan.nom_etage_exemple')} bind:value={newName} aria-label={t('maison.plan.nom_nouvel_etage')} />
        <span class="file"><Icon name="plus" size={16} />{t('maison.plan.ajouter_etage')}
          <input type="file" accept="image/png,image/jpeg,image/webp,image/svg+xml" onchange={addFloor} disabled={busy} />
        </span>
        <button class="link" onclick={addBlankFloor} disabled={busy}>{t('maison.plan.sans_image')}</button>
      </label>
      {#if floor && !floor.size}
        <span class="file small"><Icon name="camera" size={15} />{t('maison.plan.changer_image')}
          <input type="file" accept="image/png,image/jpeg,image/webp,image/svg+xml" onchange={replaceImage} disabled={busy} />
        </span>
        <button class="link danger" onclick={removeFloor}>{t('maison.plan.retirer_etage')}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .plan {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 18px;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .actions {
    display: flex;
    gap: var(--sp-2);
  }

  button.primary,
  button.ghost {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 9px 16px;
    border-radius: 999px;
    border: none;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  button.primary {
    background: var(--ink);
    color: var(--surface);
  }

  button.ghost {
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow);
  }

  .floors {
    display: flex;
    gap: 6px;
    overflow-x: auto;
  }

  .floors button {
    padding: 7px 14px;
    border-radius: 999px;
    border: none;
    background: var(--surface-2);
    color: var(--ink-2);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }

  .floors button.active {
    background: var(--ink);
    color: var(--surface);
  }

  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 16px;
  }

  .add {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    align-items: center;
  }

  .add input,
  .tray input {
    padding: 9px 12px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    min-width: 0;
  }

  .file {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 9px 14px;
    border-radius: 999px;
    background: var(--warm-soft);
    color: var(--warm-ink);
    font-weight: 600;
    cursor: pointer;
  }

  .file.small {
    padding: 6px 12px;
    font-size: var(--fs-sm);
  }

  .file input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .link {
    border: none;
    background: none;
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
  }

  .link.danger {
    color: var(--alert);
  }

  .empty {
    display: grid;
    justify-items: center;
    gap: 10px;
    padding: 40px 20px;
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--shadow);
    text-align: center;
    color: var(--ink-2);
  }

  .empty h2 {
    margin: 0;
    color: var(--ink);
  }

  .empty p {
    max-width: 46ch;
    margin: 0 0 6px;
  }

  .board {
    display: grid;
    gap: var(--sp-4);
  }

  .board.editing {
    grid-template-columns: minmax(0, 1fr) 280px;
    align-items: start;
  }

  .skins {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .skins button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink-2);
    font: inherit;
    font-size: var(--fs-sm);
    cursor: pointer;
  }

  .skins button.active {
    border-color: var(--ink);
    color: var(--ink);
    font-weight: 650;
  }

  .views {
    display: inline-flex;
    margin-right: 6px;
    padding: 3px;
    border-radius: 999px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .views button {
    padding: 5px 12px;
    border: none;
    border-radius: 999px;
    background: none;
    color: var(--ink-2);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 700;
    cursor: pointer;
  }

  .views button.active {
    background: var(--ink);
    color: var(--surface);
  }

  .size {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 6px;
  }

  .size select {
    min-height: 32px;
    padding: 4px 8px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: var(--fs-sm);
  }

  .skins i {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid;
  }

  .pick {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .seg {
    display: inline-flex;
    max-width: 100%;
    padding: 3px;
    border-radius: 999px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .seg button {
    flex: none;
    min-height: 38px;
    white-space: nowrap;
    padding: 6px 14px;
    border: none;
    border-radius: 999px;
    background: none;
    color: var(--ink-2);
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 650;
    cursor: pointer;
  }

  .seg button.active {
    background: var(--ink);
    color: var(--surface);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
  }

  /* Big enough for a finger. */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 40px;
    padding: 6px 14px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 600;
    cursor: pointer;
  }

  .chip:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .chip.danger {
    color: var(--alert);
  }

  .chip input {
    width: 18px;
    height: 18px;
    margin: 0;
  }

  .sheet {
    margin-top: 12px;
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 10px 14px;
    padding: 12px 14px;
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .sheet label {
    display: grid;
    gap: var(--sp-1);
    font-size: 12.5px;
    color: var(--ink-3);
  }

  .sheet input,
  .sheet select,
  .sheet .row select {
    min-height: 40px;
    padding: 6px 10px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 15px;
  }

  .sheet input[type='number'] {
    width: 9ch;
  }

  .sheet p {
    flex-basis: 100%;
  }

  .measure {
    flex-basis: 100%;
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 10px 14px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }

  .chip.active {
    border-color: var(--ink);
    background: var(--ink);
    color: var(--surface);
  }

  .pick select {
    padding: 6px 8px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
  }

  .whole {
    display: flex;
    gap: var(--gap);
    align-items: flex-start;
    justify-content: center;
  }

  .storey {
    flex: none;
    display: grid;
    gap: 6px;
  }

  .storey-name {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    justify-self: start;
    font-size: var(--fs-sm);
    font-weight: 650;
    color: var(--ink-2);
    cursor: pointer;
  }

  .drawn {
    min-width: 0;
  }

  .surface {
    position: relative;
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--shadow);
    overflow: hidden;
    touch-action: none;
    user-select: none;
  }

  .surface.blank {
    aspect-ratio: 4 / 3;
    background-image:
      linear-gradient(var(--line) 1px, transparent 1px),
      linear-gradient(90deg, var(--line) 1px, transparent 1px);
    background-size: 32px 32px;
  }

  .surface.picking {
    cursor: crosshair;
    outline: 3px dashed var(--warm);
    outline-offset: -3px;
  }

  .surface img {
    display: block;
    width: 100%;
    height: auto;
    pointer-events: none;
  }

  .spot {
    position: absolute;
    display: grid;
    justify-items: center;
    transform: translate(-50%, -50%);
  }

  .token {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 50%;
    border: none;
    background: var(--surface);
    color: var(--ink-2);
    box-shadow: var(--shadow-lift);
    cursor: pointer;
    transition:
      background 0.3s var(--ease),
      box-shadow 0.3s var(--ease),
      color 0.3s var(--ease);
  }

  .editing .token {
    cursor: grab;
  }

  .token.lit {
    background: var(--warm);
    color: #1c1407;
    box-shadow:
      0 0 0 6px var(--warm-glow),
      0 0 28px 6px var(--warm-glow);
  }

  .token.open {
    background: var(--cool-soft);
    color: var(--cool);
  }

  .token.alarm {
    background: var(--alert);
    color: #fff;
    animation: pulse 1.2s ease-in-out infinite;
  }

  .token.off {
    opacity: 0.45;
  }

  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 10px var(--alert-soft);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .token.alarm {
      animation: none;
    }
  }

  .reading {
    margin-top: 3px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--ink);
    font-size: 11px;
    font-weight: 700;
    box-shadow: var(--shadow);
  }

  .name {
    max-width: 92px;
    margin-top: 2px;
    overflow: hidden;
    color: var(--ink);
    font-size: 10.5px;
    font-weight: 600;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-shadow:
      0 0 3px var(--surface),
      0 0 6px var(--surface);
  }

  .unplace {
    position: absolute;
    top: -6px;
    right: -6px;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: none;
    background: var(--alert);
    color: #fff;
    cursor: pointer;
  }

  .card {
    position: absolute;
    top: 100%;
    z-index: 5;
    display: grid;
    gap: var(--sp-1);
    min-width: 180px;
    margin-top: 6px;
    padding: 12px 14px;
    border-radius: var(--r-sm);
    background: var(--surface);
    box-shadow: var(--shadow-lift);
    font-size: var(--fs-sm);
  }

  .card p {
    display: flex;
    justify-content: space-between;
    gap: var(--sp-3);
    margin: 0;
    color: var(--ink-2);
  }

  .card p b {
    color: var(--ink);
  }

  .tray {
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr);
    gap: var(--sp-2);
    max-height: 70vh;
    overflow: hidden;
    padding: var(--sp-3);
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .tray ul {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .tray button {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--r-sm);
    border: none;
    background: none;
    color: var(--ink);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .tray button small {
    margin-left: auto;
    color: var(--ink-3);
  }

  .tray button:hover,
  .tray button.chosen {
    background: var(--warm-soft);
  }

  .muted {
    color: var(--ink-3);
  }

  .small {
    font-size: 12.5px;
    margin: 0;
  }

  @media (max-width: 767.98px) {
    .board.editing {
      grid-template-columns: 1fr;
    }

    .tray {
      max-height: 40vh;
    }

    .token {
      width: 32px;
      height: 32px;
    }

    /* The four layers, two by two. */
    .seg {
      display: grid;
      grid-template-columns: 1fr 1fr;
      width: 100%;
      border-radius: 22px;
    }

    /* Undo, redo, magnet: their icons are enough on a phone. */
    .chip .word {
      display: none;
    }

    /* Icons only while looking; names while arranging. */
    .board:not(.editing) .name {
      display: none;
    }
  }
</style>
