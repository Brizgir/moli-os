<script>
  import { hub, home, value, deviceKind, isOn, roomOf, num } from '../lib/home.svelte.js';
  import { area, bbox, inside, labelAt, points } from './geometry.js';
  import * as E from './editing.js';
  import { t, locale } from '../../lib/i18n.svelte.js';

  /**
   * A drawn floor (cm): rooms, walls, openings, fixtures and the house's
   * devices, alive. Zoom (wheel, pinch, double tap, buttons) and pan (drag);
   * the skin is only colours. In edit mode, one layer at a time answers:
   * fixtures move, turn and resize; a room's corners move (handles), a side
   * gets a new one; a wall moves and stretches by its ends; an opening
   * slides along its wall. A tap chooses, a drag on what is chosen moves it
   * (a drag elsewhere still pans). The geometry is `editing.js`.
   */
  let {
    floor,
    skin,
    editing = false,
    picking = null,
    /** Edit: what is chosen, `{ type: 'item' | 'room' | 'wall' | 'opening', i }`. */
    selection = $bindable(null),
    /** Edit: the chosen corner of the chosen room. */
    vertex = $bindable(null),
    /** Edit: which layer answers ('items' with the devices, 'rooms', 'walls', 'openings'). */
    layer = 'items',
    /** Edit: the grid and the lines around catch what moves. */
    magnet = true,
    /** Edit: the next tap picks the reference to measure from (any layer). */
    aiming = false,
    /** Edit: the reference measured from, `{ type, i }`; the cotes go from it
     *  to what is chosen. */
    ruler = null,
    onpick = () => {},
    /** How big the devices and the temperatures show (1: as drawn). */
    sizes = { devices: 1, gauges: 1 },
    /** Edit: a change starts (to undo it), and ends. */
    onbegin = () => {},
    onend = () => {},
    token,
    keep = () => true,
    onplace = () => {},
    ontap = () => {},
    /** Fills its box (the overview sets the size) instead of keeping the floor's shape. */
    fill = false,
  } = $props();

  const uid = Math.random().toString(36).slice(2, 8);
  let box = $state(null);
  let W = $state(0);
  let H = $state(0);
  let cam = $state({ x: 0, y: 0, k: 1 });
  let fitK = $state(1);

  const size = $derived(floor.size ?? [1000, 1000]);

  /** The whole floor in view. */
  export function fit() {
    if (!W || !H) return;
    const [w, h] = size;
    const k = Math.min(W / w, H / h) * 0.97;
    fitK = k;
    cam = { k, x: (w - W / k) / 2, y: (h - H / k) / 2 };
  }

  // A new floor, or a new screen size: start from the whole floor.
  let fitted = '';
  $effect(() => {
    const key = `${floor.id}:${W}x${H}`;
    if (W && H && key !== fitted) {
      fitted = key;
      fit();
    }
  });

  /** Screen → floor (cm). */
  export function toWorld(clientX, clientY) {
    const r = box.getBoundingClientRect();
    return { x: cam.x + (clientX - r.left) / cam.k, y: cam.y + (clientY - r.top) / cam.k };
  }

  /** The middle of what is on screen (cm). */
  export function center() {
    return { x: cam.x + W / cam.k / 2, y: cam.y + H / cam.k / 2 };
  }

  function zoomAt(factor, clientX, clientY) {
    const k = Math.min(fitK * 14, Math.max(fitK * 0.6, cam.k * factor));
    const p = toWorld(clientX, clientY);
    const r = box.getBoundingClientRect();
    cam = { k, x: p.x - (clientX - r.left) / k, y: p.y - (clientY - r.top) / k };
  }

  export function zoom(factor) {
    const r = box.getBoundingClientRect();
    zoomAt(factor, r.left + W / 2, r.top + H / 2);
  }

  // ---- gestures -------------------------------------------------------------------

  const pointers = new Map();
  let start = null;
  let pinch = null;
  let moved = false;

  function down(e) {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 1) {
      moved = false;
      start = { x: e.clientX, y: e.clientY, cam: { ...cam } };
    } else if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      pinch = { d: Math.hypot(a.x - b.x, a.y - b.y) || 1, k: cam.k, at: toWorld(mid.x, mid.y) };
      moved = true;
    }
  }

  function move(e) {
    if (hold && hold.id === e.pointerId) {
      if (!steady(e)) {
        const p = toWorld(e.clientX, e.clientY);
        hold.apply(p.x, p.y);
      }
      return;
    }
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size >= 2 && pinch) {
      const [a, b] = [...pointers.values()];
      const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      const k = Math.min(fitK * 14, Math.max(fitK * 0.6, (pinch.k * Math.hypot(a.x - b.x, a.y - b.y)) / pinch.d));
      const r = box.getBoundingClientRect();
      cam = { k, x: pinch.at.x - (mid.x - r.left) / k, y: pinch.at.y - (mid.y - r.top) / k };
      return;
    }
    if (!start) return;
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    if (!moved && Math.abs(dx) + Math.abs(dy) < 6) return;
    if (!moved) {
      moved = true;
      box.setPointerCapture?.(e.pointerId);
    }
    cam = { k: start.cam.k, x: start.cam.x - dx / start.cam.k, y: start.cam.y - dy / start.cam.k };
  }

  function up(e) {
    if (hold && hold.id === e.pointerId) {
      hold = null;
      onend();
      return;
    }
    if (!pointers.has(e.pointerId)) return;
    pointers.delete(e.pointerId);
    if (pointers.size < 2) pinch = null;
    if (pointers.size === 1) {
      const [p] = [...pointers.values()];
      start = { x: p.x, y: p.y, cam: { ...cam } };
      return;
    }
    if (pointers.size === 0) {
      if (!moved) {
        const p = toWorld(e.clientX, e.clientY);
        if (editing && picking) onplace(Math.round(p.x), Math.round(p.y));
        else if (editing && aiming) onpick(pickOf(e.target.closest?.('[data-pick]')));
        else if (editing) choose(e.target.closest?.('[data-pick]'));
        else ontap(p);
      }
      start = null;
    }
  }

  $effect(() => {
    if (!box) return;
    const wheel = (e) => {
      e.preventDefault();
      zoomAt(Math.exp(-e.deltaY * 0.0015), e.clientX, e.clientY);
    };
    box.addEventListener('wheel', wheel, { passive: false });
    return () => box.removeEventListener('wheel', wheel);
  });

  function dbl(e) {
    if (e.target.closest?.('.pin, .handle, .hit')) return;
    zoomAt(2, e.clientX, e.clientY);
  }

  // ---- edit: choose with a tap, then move what is chosen ------------------------------

  /** A drag under way: `apply(x, y)` (cm) until the finger lifts. */
  let hold = null;
  /** How close (cm) a line catches what moves, how big a handle is: pixels on screen. */
  const tol = $derived(12 / cam.k);
  const hit = $derived(14 / cam.k);
  const LISTS = { item: 'items', room: 'rooms', wall: 'walls', opening: 'openings' };
  const is = (type, i) => editing && selection?.type === type && selection.i === i;

  const pickOf = (el) => (el ? { type: el.dataset.pick, i: Number(el.dataset.i) } : null);

  function choose(el) {
    selection = pickOf(el);
    vertex = null;
  }

  /** The box a thing covers (a room: around its corners). */
  function boxOf(sel) {
    const s = sel ? floor[LISTS[sel.type]]?.[sel.i] : null;
    if (!s) return null;
    return sel.type === 'room' ? bbox(s.poly) : E.footprint(s);
  }
  const refBox = $derived(editing && ruler ? boxOf(ruler) : null);
  const selBox = $derived(editing && selection && refBox && !(selection.type === ruler.type && selection.i === ruler.i) ? boxOf(selection) : null);
  const cotes = $derived(refBox && selBox ? E.gaps(refBox, selBox) : null);
  /** Where a cote shows, in cm. */
  const cote = (axis) => {
    const c = cotes[axis];
    const a = refBox;
    const b = selBox;
    if (axis === 'x') {
      const y = b.y + b.h / 2;
      return c.after ? { x1: a.x + a.w, x2: b.x, y1: y, y2: y } : { x1: b.x + b.w, x2: a.x, y1: y, y2: y };
    }
    const x = b.x + b.w / 2;
    return c.after ? { x1: x, x2: x, y1: a.y + a.h, y2: b.y } : { x1: x, x2: x, y1: b.y + b.h, y2: a.y };
  };

  function grab(e, apply = () => {}) {
    e.stopPropagation();
    try {
      box.setPointerCapture?.(e.pointerId);
    } catch {
      // A pointer already gone: the drag ends with it.
    }
    onbegin();
    hold = { id: e.pointerId, sx: e.clientX, sy: e.clientY, apply, moved: false };
    return hold;
  }

  /** Still while the finger hesitates: a tap chooses, it does not nudge. */
  function steady(e) {
    if (!hold.moved && Math.abs(e.clientX - hold.sx) + Math.abs(e.clientY - hold.sy) < 4) return true;
    hold.moved = true;
    return false;
  }

  const snap = (v) => (magnet ? E.toGrid(v) : Math.round(v));

  function grabItem(e, i, mode = 'move') {
    if (!editing || layer !== 'items' || aiming) return;
    selection = { type: 'item', i };
    const start = $state.snapshot(floor.items[i]);
    const p0 = toWorld(e.clientX, e.clientY);
    grab(e, (x, y) => {
      const it = floor.items[i];
      const dx = x - p0.x;
      const dy = y - p0.y;
      if (mode === 'move') {
        Object.assign(it, E.clampBox({ ...it, x: snap(start.x + dx), y: snap(start.y + dy) }, size));
        return;
      }
      // Resize in the fixture's own frame (it may be turned).
      const a = (-(start.r ?? 0) * Math.PI) / 180;
      const lx = dx * Math.cos(a) - dy * Math.sin(a);
      const ly = dx * Math.sin(a) + dy * Math.cos(a);
      it.w = Math.max(10, snap(start.w + lx));
      it.h = Math.max(10, snap(start.h + ly));
    });
  }

  /** The pill hangs this far (cm) above the point it is anchored to. */
  const LIFT = $derived(18 / cam.k);

  /** The temperature pill of room `r` following the finger (anywhere on
   *  the floor); the handle sits on the pill, not on its anchor. */
  function grabGauge(e, r) {
    grab(e, (x, y) => {
      floor.rooms[r].gauge = E.clampPoint([x, y + LIFT], size).map((v) => (magnet ? E.toGrid(v) : Math.round(v)));
    });
  }

  /** Corner `j` of room `r` following the finger, caught by its neighbours
   *  (right angles), the other rooms' corners and the walls' faces. */
  function cornerMover(r, j) {
    const guides = E.mergeGuides(E.roomGuides(floor.rooms, [r, j]), E.wallGuides(floor.walls ?? []));
    return (x, y) => {
      const p = E.clampPoint([x, y], size);
      floor.rooms[r].poly[j] = magnet ? E.snapPoint(p, guides, tol) : p.map(Math.round);
    };
  }

  function grabCorner(e, r, j) {
    vertex = j;
    grab(e, cornerMover(r, j));
  }

  /** A new corner in the middle of side `j`, held at once. */
  function grabSide(e, r, j) {
    const room = floor.rooms[r];
    if (room.poly.length >= E.MAX_POINTS) return;
    const h = grab(e);
    room.poly = E.insertVertex($state.snapshot(room.poly), j);
    vertex = j + 1;
    h.apply = cornerMover(r, j + 1);
  }

  /** A wall moved whole (`end` null) or by one end; its openings follow. */
  function grabWall(e, i, end = null) {
    const h = grab(e);
    // A wall turned by a quarter becomes the plain box it covers.
    const start = E.unturned($state.snapshot(floor.walls[i]));
    floor.walls[i] = start;
    const flat = E.straight(start) && start.r == null;
    const openings = $state.snapshot(floor.openings ?? []);
    const carried = flat ? E.openingsOn(start, openings).map((k) => [k, openings[k]]) : [];
    const guides = flat && magnet ? E.wallGuides(floor.walls, i) : null;
    const p0 = toWorld(e.clientX, e.clientY);
    h.apply = (x, y) => {
      const next =
        end == null
          ? E.clampBox(E.moveWall(start, x - p0.x, y - p0.y, guides, tol), size)
          : E.stretchWall(start, end, E.clampPoint([x, y], size), guides, tol);
      floor.walls[i] = next;
      const [dx, dy] = end == null ? [next.x - start.x, next.y - start.y] : [0, 0];
      for (const [k, o] of carried) floor.openings[k] = E.carryOpening(o, start, next, dx, dy);
    };
  }

  /** An opening sliding along the closest wall. */
  function grabOpening(e, i) {
    const h = grab(e);
    const start = $state.snapshot(floor.openings[i]);
    const walls = $state.snapshot(floor.walls ?? []);
    const width = E.widthOf(start, walls);
    const p0 = toWorld(e.clientX, e.clientY);
    const [cx, cy] = E.centerOf(start);
    h.apply = (x, y) => {
      const [px, py] = E.clampPoint([x - (p0.x - cx), y - (p0.y - cy)], size);
      floor.openings[i] = E.placeOpening(walls, start, width, px, py, magnet);
    };
  }

  // ---- what is alive -------------------------------------------------------------------

  const LIGHTS = ['light', 'group'];

  function level(id) {
    const b = value(id, 'brightness');
    return typeof b === 'number' ? Math.max(0.15, Math.min(1, b / 100)) : 1;
  }

  /** Lit lights placed on this floor: where they shine, how much, in which room. */
  const glows = $derived(
    (floor.devices ?? [])
      .map((s) => ({ s, d: hub.devices[s.id] }))
      .filter(({ d }) => d && LIGHTS.includes(deviceKind(d)) && isOn(d.id))
      .map(({ s, d }) => ({
        id: s.id,
        x: s.x,
        y: s.y,
        level: level(d.id),
        room: (floor.rooms ?? []).find((r) => inside(r.poly, s.x, s.y))?.id ?? null,
      })),
  );

  /** Rooms whose Moli room has a light on (placed or not): a warm tint. */
  const warm = $derived.by(() => {
    const on = new Map();
    for (const d of Object.values(hub.devices)) {
      if (!LIGHTS.includes(deviceKind(d)) || !isOn(d.id)) continue;
      const room = roomOf(d);
      if (room) on.set(room, Math.max(on.get(room) ?? 0, level(d.id)));
    }
    return on;
  });

  const label = (n) => n.replace(/\s+/g, ' ');
  /** Devices small when the whole floor shows, full size once zoomed in. */
  const pinScale = $derived(Math.min(1, Math.max(0.6, (cam.k / fitK) * 0.6)) * (sizes.devices ?? 1));
  const fs = $derived(12 / cam.k);
  const visible = (x, y) => x > cam.x - 80 / cam.k && y > cam.y - 80 / cam.k && x < cam.x + (W + 80) / cam.k && y < cam.y + (H + 80) / cam.k;

  // ---- the temperature of each place ------------------------------------------------

  /** A probe's temperature now (an unreachable one says nothing). */
  function tempOf(id) {
    const d = id ? hub.devices[id] : null;
    if (!d || d.online === false) return null;
    for (const key of ['temperature', 'temp_current']) {
      const v = value(id, key);
      if (typeof v === 'number') return v;
    }
    return null;
  }

  /** Where a room's temperature sits unless told: its foot, under its label. */
  function gaugeAt0(poly) {
    const [x] = labelAt(poly);
    return [x, bottomAt(poly, x)];
  }

  /** The lowest point of a room under x: where its temperature sits. */
  function bottomAt(poly, x) {
    const b = bbox(poly);
    for (let i = 1; i <= 40; i++) {
      const y = b.y + b.h - (b.h * i) / 40;
      if (inside(poly, x, y)) return y;
    }
    return b.y + b.h;
  }

  const INDOOR = new Set(['living', 'kitchen', 'bedroom', 'bath', 'wc', 'office', 'hall', 'veranda', 'storage', 'other']);

  /** Each room's temperature, from the probes placed in it (averaged);
   *  the garden without one takes the weather's. Shown at the room's
   *  foot, not where the probe hangs. */
  const climates = $derived.by(() => {
    const outside = tempOf(home.config?.outdoor?.weather);
    let garden = false;
    const out = [];
    const rooms = (floor.rooms ?? []).filter((r) => r.kind !== 'void' && r.kind !== 'roof');
    // A probe counts for the smallest room around it (the pool, not the
    // deck around the pool).
    const owner = new Map();
    for (const s of floor.devices ?? []) {
      const around = rooms.filter((r) => inside(r.poly, s.x, s.y));
      if (around.length) owner.set(s.id, around.reduce((a, b) => (area(b.poly) < area(a.poly) ? b : a)).id);
    }
    for (const r of rooms) {
      const temps = (floor.devices ?? []).filter((s) => owner.get(s.id) === r.id).map((s) => tempOf(s.id)).filter((t) => t != null);
      let t = temps.length ? temps.reduce((a, b) => a + b, 0) / temps.length : null;
      if (t == null && r.kind === 'garden' && !garden) t = outside;
      if (r.kind === 'garden') garden = true;
      if (t == null) continue;
      const [x, y] = r.gauge ?? gaugeAt0(r.poly);
      out.push({ id: r.id, t, x, y, w: bbox(r.poly).w, indoor: INDOOR.has(r.kind) });
    }
    return out;
  });

  /** The floor's own: the mean of its rooms (indoor ones). */
  const floorTemp = $derived.by(() => {
    const rooms = climates.filter((c) => c.indoor);
    return rooms.length ? rooms.reduce((a, c) => a + c.t, 0) / rooms.length : null;
  });

  /** Cold to hot, at a glance. */
  const tone = (t) => (t < 18 ? 'cold' : t < 20 ? 'cool' : t <= 24 ? 'good' : t <= 27 ? 'warm' : 'hot');
  const gaugeAt = (t) => Math.min(100, Math.max(0, ((t - 14) / 16) * 100));

  const TEXTURE = { garden: 'grass', pool: 'water', terrace: 'planks', deck: 'stone' };
  const dark = $derived(skin.bg.startsWith('#0') || skin.bg.startsWith('#1'));
</script>

{#snippet editLayer()}
  {@const sel = selection ? floor[LISTS[selection.type]]?.[selection.i] : null}
  {@const line = { stroke: skin.glow, 'stroke-width': 2.5, 'vector-effect': 'non-scaling-stroke', fill: 'none' }}
  <g class="edit">
    <!-- the layer's hit boxes: thin walls, easy to touch -->
    {#if aiming}
      <!-- any wall, opening or fixture may be the reference (rooms answer by themselves) -->
      {#each floor.walls ?? [] as w, i (i)}
        <rect class="hit" data-pick="wall" data-i={i} x={w.x - hit} y={w.y - hit} width={w.w + 2 * hit} height={w.h + 2 * hit} transform={w.r ? `rotate(${w.r} ${w.x + w.w / 2} ${w.y + w.h / 2})` : null} />
      {/each}
      {#each floor.openings ?? [] as o, i (i)}
        <rect class="hit" data-pick="opening" data-i={i} x={o.x - hit} y={o.y - hit} width={o.w + 2 * hit} height={o.h + 2 * hit} />
      {/each}
      {#each floor.items ?? [] as it, i (i)}
        <rect class="hit" data-pick="item" data-i={i} x={it.x} y={it.y} width={it.w} height={it.h} transform={it.r ? `rotate(${it.r} ${it.x + it.w / 2} ${it.y + it.h / 2})` : null} />
      {/each}
    {:else if layer === 'walls'}
      {#each floor.walls ?? [] as w, i (i)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <rect class="hit" data-pick="wall" data-i={i} x={w.x - hit} y={w.y - hit} width={w.w + 2 * hit} height={w.h + 2 * hit} transform={w.r ? `rotate(${w.r} ${w.x + w.w / 2} ${w.y + w.h / 2})` : null} onpointerdown={(e) => is('wall', i) && grabWall(e, i)} />
      {/each}
    {:else if layer === 'openings'}
      {#each floor.openings ?? [] as o, i (i)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <rect class="hit" data-pick="opening" data-i={i} x={o.x - hit} y={o.y - hit} width={o.w + 2 * hit} height={o.h + 2 * hit} onpointerdown={(e) => is('opening', i) && grabOpening(e, i)} />
      {/each}
    {/if}

    {#if refBox}
      <!-- the reference, and the cotes from it to what is chosen -->
      <rect x={refBox.x - 3 / cam.k} y={refBox.y - 3 / cam.k} width={refBox.w + 6 / cam.k} height={refBox.h + 6 / cam.k} fill="none" stroke={skin.window} stroke-width="2.5" stroke-dasharray="2 4" vector-effect="non-scaling-stroke" pointer-events="none" />
      {#if cotes}
        {#each ['x', 'y'] as axis (axis)}
          {#if cotes[axis].gap > 0}
            {@const l = cote(axis)}
            <g class="cote" pointer-events="none">
              <line x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} stroke={skin.label} stroke-width="1.5" vector-effect="non-scaling-stroke" />
              {#if axis === 'x'}
                <line x1={l.x1} x2={l.x1} y1={l.y1 - 8 / cam.k} y2={l.y1 + 8 / cam.k} stroke={skin.label} stroke-width="1.5" vector-effect="non-scaling-stroke" />
                <line x1={l.x2} x2={l.x2} y1={l.y1 - 8 / cam.k} y2={l.y1 + 8 / cam.k} stroke={skin.label} stroke-width="1.5" vector-effect="non-scaling-stroke" />
                <text class="label" x={(l.x1 + l.x2) / 2} y={l.y1 - 6 / cam.k} font-size={fs} fill={skin.label} stroke={skin.bg} stroke-width={3 / cam.k} text-anchor="middle">{cotes.x.gap} cm</text>
              {:else}
                <line y1={l.y1} y2={l.y1} x1={l.x1 - 8 / cam.k} x2={l.x1 + 8 / cam.k} stroke={skin.label} stroke-width="1.5" vector-effect="non-scaling-stroke" />
                <line y1={l.y2} y2={l.y2} x1={l.x1 - 8 / cam.k} x2={l.x1 + 8 / cam.k} stroke={skin.label} stroke-width="1.5" vector-effect="non-scaling-stroke" />
                <text class="label" x={l.x1 + 6 / cam.k} y={(l.y1 + l.y2) / 2 + fs * 0.35} font-size={fs} fill={skin.label} stroke={skin.bg} stroke-width={3 / cam.k}>{cotes.y.gap} cm</text>
              {/if}
            </g>
          {/if}
        {/each}
      {/if}
    {/if}

    {#if aiming}
      <!-- the handles wait: the tap picks the reference -->
    {:else if sel && selection.type === 'room'}
      <polygon points={points(sel.poly)} {...line} pointer-events="none" />
      {#if sel.kind !== 'void' && sel.kind !== 'roof'}
        {@const [gx, gy0] = sel.gauge ?? gaugeAt0(sel.poly)}
        {@const gy = gy0 - LIFT}
        <!-- where its temperature shows: a thermometer to drag -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <g class="handle gauge-handle" onpointerdown={(e) => grabGauge(e, selection.i)}>
          <circle cx={gx} cy={gy} r={hit * 1.2} fill="transparent" />
          <circle cx={gx} cy={gy} r={11 / cam.k} fill={skin.bg} stroke={skin.glow} stroke-width="2" vector-effect="non-scaling-stroke" />
          <path d="M {gx} {gy - 6 / cam.k} v {8 / cam.k}" stroke={skin.glow} stroke-width="3" stroke-linecap="round" vector-effect="non-scaling-stroke" />
          <circle cx={gx} cy={gy + 3 / cam.k} r={3 / cam.k} fill={skin.glow} />
        </g>
      {/if}
      {#if sel.poly.length < E.MAX_POINTS}
        {#each sel.poly as [x, y], j (j)}
          {@const [nx, ny] = sel.poly[(j + 1) % sel.poly.length]}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <g class="handle" onpointerdown={(e) => grabSide(e, selection.i, j)}>
            <circle cx={(x + nx) / 2} cy={(y + ny) / 2} r={hit * 1.3} fill="transparent" />
            <circle cx={(x + nx) / 2} cy={(y + ny) / 2} r={7 / cam.k} fill={skin.bg} stroke={skin.glow} stroke-width="2" vector-effect="non-scaling-stroke" />
            <path d="M {(x + nx) / 2 - 4 / cam.k} {(y + ny) / 2} h {8 / cam.k} M {(x + nx) / 2} {(y + ny) / 2 - 4 / cam.k} v {8 / cam.k}" stroke={skin.glow} stroke-width="2" vector-effect="non-scaling-stroke" />
          </g>
        {/each}
      {/if}
      {#each sel.poly as [x, y], j (j)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <g class="handle" onpointerdown={(e) => grabCorner(e, selection.i, j)}>
          <circle cx={x} cy={y} r={hit * 1.6} fill="transparent" />
          <circle cx={x} cy={y} r={(vertex === j ? 11 : 8) / cam.k} fill={skin.glow} stroke={skin.bg} stroke-width="2" vector-effect="non-scaling-stroke" />
        </g>
      {/each}
    {:else if sel && selection.type === 'wall'}
      {@const flat = sel.r == null || E.straight(sel)}
      {@const [cx, cy] = E.centerOf(sel)}
      <rect x={sel.x - 4 / cam.k} y={sel.y - 4 / cam.k} width={sel.w + 8 / cam.k} height={sel.h + 8 / cam.k} transform={sel.r ? `rotate(${sel.r} ${cx} ${cy})` : null} {...line} stroke-dasharray="6 4" pointer-events="none" />
      {#if flat}
        {#each E.ends(E.straight(sel)) as [x, y], end (end)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <g class="handle" onpointerdown={(e) => grabWall(e, selection.i, end)}>
            <circle cx={x} cy={y} r={hit * 1.6} fill="transparent" />
            <circle cx={x} cy={y} r={8 / cam.k} fill={skin.glow} stroke={skin.bg} stroke-width="2" vector-effect="non-scaling-stroke" />
          </g>
        {/each}
      {/if}
      {@render mover(cx, cy, (e) => grabWall(e, selection.i))}
    {:else if sel && selection.type === 'opening'}
      {@const [cx, cy] = E.centerOf(sel)}
      <rect x={sel.x - 4 / cam.k} y={sel.y - 4 / cam.k} width={sel.w + 8 / cam.k} height={sel.h + 8 / cam.k} {...line} stroke-dasharray="6 4" pointer-events="none" />
      {@render mover(cx, cy, (e) => grabOpening(e, selection.i))}
    {/if}
  </g>
{/snippet}

<!-- A handle in the middle of what is chosen: hold it to move it. -->
{#snippet mover(cx, cy, ondown)}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <g class="handle" onpointerdown={ondown}>
    <circle cx={cx} cy={cy} r={hit * 1.6} fill="transparent" />
    <circle cx={cx} cy={cy} r={11 / cam.k} fill={skin.glow} stroke={skin.bg} stroke-width="2" vector-effect="non-scaling-stroke" />
    <path d="M {cx - 6 / cam.k} {cy} h {12 / cam.k} M {cx} {cy - 6 / cam.k} v {12 / cam.k}" stroke={skin.bg} stroke-width="2" vector-effect="non-scaling-stroke" />
  </g>
{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="drawing"
  class:fill
  class:picking={!!picking || aiming}
  bind:this={box}
  bind:clientWidth={W}
  bind:clientHeight={H}
  style="aspect-ratio: {size[0]} / {size[1]}; background: {skin.bg}"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  ondblclick={dbl}
>
  {#if W && H}
    <svg width={W} height={H} viewBox="{cam.x} {cam.y} {W / cam.k} {H / cam.k}" role="img" aria-label={t('maison.plan.nom_etage', { name: floor.name })}>
      <defs>
        {#if skin.grid}
          <pattern id="grid-{uid}" width="100" height="100" patternUnits="userSpaceOnUse">
            <path d="M 100 0 L 0 0 0 100" fill="none" stroke={skin.grid} stroke-width="2" />
          </pattern>
        {/if}
        <pattern id="grass-{uid}" width="60" height="60" patternUnits="userSpaceOnUse">
          <circle cx="12" cy="14" r="3" fill={dark ? '#2b4a2e' : '#9fbf86'} opacity="0.55" />
          <circle cx="42" cy="38" r="2.5" fill={dark ? '#2b4a2e' : '#9fbf86'} opacity="0.45" />
          <circle cx="30" cy="52" r="2" fill={dark ? '#2b4a2e' : '#9fbf86'} opacity="0.4" />
        </pattern>
        <pattern id="water-{uid}" width="80" height="36" patternUnits="userSpaceOnUse">
          <path d="M0 18 Q 20 8 40 18 T 80 18" fill="none" stroke="#ffffff" stroke-opacity="0.35" stroke-width="3" />
        </pattern>
        <pattern id="planks-{uid}" width="200" height="24" patternUnits="userSpaceOnUse">
          <path d="M0 24 H200 M120 0 V24" stroke={dark ? '#000' : '#b89b74'} stroke-opacity="0.35" stroke-width="2" />
        </pattern>
        <pattern id="stone-{uid}" width="50" height="50" patternUnits="userSpaceOnUse">
          <path d="M0 50 H50 V0" fill="none" stroke={dark ? '#000' : '#bdb5a6'} stroke-opacity="0.4" stroke-width="2" />
        </pattern>
        <radialGradient id="glow-{uid}">
          <stop offset="0" stop-color={skin.glow} stop-opacity="1" />
          <stop offset="0.55" stop-color={skin.glow} stop-opacity="0.35" />
          <stop offset="1" stop-color={skin.glow} stop-opacity="0" />
        </radialGradient>
        {#each floor.rooms ?? [] as r (r.id)}
          <clipPath id="clip-{uid}-{r.id}"><polygon points={points(r.poly)} /></clipPath>
        {/each}
      </defs>

      {#if skin.grid}
        <rect x={-2000} y={-2000} width={size[0] + 4000} height={size[1] + 4000} fill="url(#grid-{uid})" />
      {/if}

      <!-- rooms, their texture, their warmth -->
      {#each floor.rooms ?? [] as r, i (r.id)}
        <polygon class="room" data-pick={editing && (layer === 'rooms' || aiming) ? 'room' : null} data-i={i} points={points(r.poly)} fill={skin.rooms[r.kind] ?? skin.rooms.other} stroke={skin.roomLine} stroke-width="1.2" vector-effect="non-scaling-stroke" stroke-dasharray={skin.roomLine === 'none' ? null : '6 4'} />
        {#if TEXTURE[r.kind]}
          <polygon class="texture" points={points(r.poly)} fill="url(#{TEXTURE[r.kind]}-{uid})" />
        {/if}
        {#if r.room && warm.has(r.room)}
          <polygon class="warm" points={points(r.poly)} fill={skin.glow} opacity={0.1 * skin.glowStrength * warm.get(r.room)} />
        {/if}
      {/each}

      <!-- each lit light shines in its room -->
      {#each glows as g (g.id)}
        <circle
          class="glow"
          cx={g.x}
          cy={g.y}
          r={150 + 170 * g.level}
          fill="url(#glow-{uid})"
          opacity={skin.glowStrength * (0.45 + 0.55 * g.level)}
          clip-path={g.room ? `url(#clip-${uid}-${g.room})` : null}
        />
      {/each}

      <!-- fixtures -->
      {#each floor.items ?? [] as it, i (i)}
        {@const cx = it.x + it.w / 2}
        {@const cy = it.y + it.h / 2}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <g
          class="item"
          class:chosen={is('item', i)}
          class:asleep={editing && layer !== 'items'}
          transform="rotate({it.r ?? 0} {cx} {cy})"
          onpointerdown={(e) => grabItem(e, i)}
        >
          {#if it.kind === 'plant'}
            <circle {cx} {cy} r={Math.min(it.w, it.h) / 2} fill={dark ? '#1f3323' : '#cfe3bf'} stroke={skin.itemLine} stroke-width="1.2" vector-effect="non-scaling-stroke" />
          {:else if it.kind === 'trampoline'}
            <ellipse {cx} {cy} rx={it.w / 2} ry={it.h / 2} fill={skin.item} stroke={skin.itemLine} stroke-width="1.2" vector-effect="non-scaling-stroke" />
            <ellipse {cx} {cy} rx={it.w * 0.42} ry={it.h * 0.42} fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
          {:else if it.kind === 'toilet' || it.kind === 'sink'}
            <ellipse {cx} {cy} rx={it.w / 2} ry={it.h / 2} fill={skin.item} stroke={skin.itemLine} stroke-width="1.2" vector-effect="non-scaling-stroke" />
          {:else}
            <rect x={it.x} y={it.y} width={it.w} height={it.h} rx={Math.min(it.w, it.h) * (it.kind === 'car' ? 0.3 : 0.08)} fill={skin.item} stroke={skin.itemLine} stroke-width="1.2" vector-effect="non-scaling-stroke" />
            {#if it.kind === 'stairs'}
              {@const steps = Math.max(2, Math.round(Math.max(it.w, it.h) / 26))}
              {#each Array(steps - 1) as _, n (n)}
                {#if it.h >= it.w}
                  <line x1={it.x} x2={it.x + it.w} y1={it.y + (it.h * (n + 1)) / steps} y2={it.y + (it.h * (n + 1)) / steps} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
                {:else}
                  <line y1={it.y} y2={it.y + it.h} x1={it.x + (it.w * (n + 1)) / steps} x2={it.x + (it.w * (n + 1)) / steps} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
                {/if}
              {/each}
            {:else if it.kind === 'bed'}
              <rect x={it.x + it.w * 0.08} y={it.y + it.h * 0.05} width={it.w * 0.38} height={it.h * 0.16} rx="6" fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <rect x={it.x + it.w * 0.54} y={it.y + it.h * 0.05} width={it.w * 0.38} height={it.h * 0.16} rx="6" fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x} x2={it.x + it.w} y1={it.y + it.h * 0.32} y2={it.y + it.h * 0.32} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'sofa'}
              <rect x={it.x} y={it.y} width={it.w} height={it.h * 0.28} fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x + it.w * 0.14} x2={it.x + it.w * 0.14} y1={it.y + it.h * 0.28} y2={it.y + it.h} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x + it.w * 0.86} x2={it.x + it.w * 0.86} y1={it.y + it.h * 0.28} y2={it.y + it.h} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'car'}
              <line x1={it.x + it.w * 0.12} x2={it.x + it.w * 0.88} y1={it.y + it.h * 0.27} y2={it.y + it.h * 0.27} stroke={skin.itemLine} stroke-width="1.5" vector-effect="non-scaling-stroke" />
              <line x1={it.x + it.w * 0.12} x2={it.x + it.w * 0.88} y1={it.y + it.h * 0.8} y2={it.y + it.h * 0.8} stroke={skin.itemLine} stroke-width="1.5" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'bunk'}
              <rect x={it.x + it.w * 0.15} y={it.y + it.h * 0.05} width={it.w * 0.7} height={it.h * 0.14} rx="6" fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x} x2={it.x + it.w} y1={it.y + it.h * 0.25} y2={it.y + it.h * 0.25} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x} x2={it.x + it.w} y1={it.y} y2={it.y + it.h} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" stroke-dasharray="4 4" />
            {:else if it.kind === 'armchair'}
              <rect x={it.x} y={it.y} width={it.w} height={it.h * 0.26} fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x + it.w * 0.2} x2={it.x + it.w * 0.2} y1={it.y + it.h * 0.26} y2={it.y + it.h} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              <line x1={it.x + it.w * 0.8} x2={it.x + it.w * 0.8} y1={it.y + it.h * 0.26} y2={it.y + it.h} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'chair'}
              <line x1={it.x} x2={it.x + it.w} y1={it.y + it.h * 0.18} y2={it.y + it.h * 0.18} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'desk'}
              <line x1={it.x + it.w * 0.3} x2={it.x + it.w * 0.7} y1={it.y + it.h * 0.2} y2={it.y + it.h * 0.2} stroke={skin.itemLine} stroke-width="2.5" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'tv'}
              <line x1={it.x + it.w * 0.07} x2={it.x + it.w * 0.93} y1={it.y + it.h * 0.2} y2={it.y + it.h * 0.2} stroke={skin.itemLine} stroke-width="2.5" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'washer'}
              <circle {cx} {cy} r={Math.min(it.w, it.h) * 0.3} fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'fridge' || it.kind === 'bin'}
              <line x1={it.x} x2={it.x + it.w} y1={it.y + it.h * 0.15} y2={it.y + it.h * 0.15} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {:else if it.kind === 'lounger'}
              {#each Array(Math.max(2, Math.round(it.h / 14))) as _, n (n)}
                <line x1={it.x} x2={it.x + it.w} y1={it.y + 7 + n * 14} y2={it.y + 7 + n * 14} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              {/each}
            {:else if it.kind === 'stairs_u'}
              {@const turn = Math.min(it.h / 2, it.w * 0.4)}
              {@const run = it.w - turn}
              {@const n = Math.max(2, Math.round(run / 26))}
              <line x1={it.x} x2={it.x + run} y1={cy} y2={cy} stroke={skin.itemLine} stroke-width="1.6" vector-effect="non-scaling-stroke" />
              {#each Array(n - 1) as _, k (k)}
                <line x1={it.x + (run * (k + 1)) / n} x2={it.x + (run * (k + 1)) / n} y1={it.y} y2={it.y + it.h} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              {/each}
              {#each [0.2, 0.4, 0.6, 0.8] as t (t)}
                <line x1={it.x + run} y1={cy} x2={it.x + it.w} y2={it.y + it.h * t} stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
              {/each}
            {:else if it.kind === 'bathtub'}
              <rect x={it.x + it.w * 0.08} y={it.y + it.h * 0.12} width={it.w * 0.84} height={it.h * 0.76} rx={it.h * 0.3} fill="none" stroke={skin.itemLine} stroke-width="1" vector-effect="non-scaling-stroke" />
            {/if}
          {/if}
          {#if is('item', i)}
            <rect class="frame" x={it.x - 6} y={it.y - 6} width={it.w + 12} height={it.h + 12} fill="none" stroke={skin.glow} stroke-width="2" stroke-dasharray="6 4" vector-effect="non-scaling-stroke" />
            <circle
              class="handle"
              cx={it.x + it.w}
              cy={it.y + it.h}
              r={hit}
              fill={skin.glow}
              onpointerdown={(e) => grabItem(e, i, 'size')}
            />
          {/if}
        </g>
      {/each}

      <!-- walls, then what opens in them -->
      <g class:asleep={editing}>
      {#each floor.walls ?? [] as w, i (i)}
        <rect x={w.x} y={w.y} width={w.w} height={w.h} fill={skin.wall} transform={w.r ? `rotate(${w.r} ${w.x + w.w / 2} ${w.y + w.h / 2})` : null} opacity={w.height && w.height < 120 ? 0.55 : null} />
      {/each}
      {#each floor.openings ?? [] as o, i (i)}
        {@const along = o.w >= o.h}
        {@const fill = skin.rooms.hall}
        <g class="opening">
          <rect x={o.x} y={o.y} width={o.w} height={o.h} fill={o.kind === 'door' ? fill : o.kind === 'gate' ? skin.door : skin.bg} />
          {#if o.kind === 'window' || o.kind === 'bay'}
            <rect x={o.x} y={o.y} width={o.w} height={o.h} fill={skin.window} opacity="0.22" />
            {#if along}
              <line x1={o.x} x2={o.x + o.w} y1={o.y + o.h / 2} y2={o.y + o.h / 2} stroke={skin.window} stroke-width="1.6" vector-effect="non-scaling-stroke" />
            {:else}
              <line y1={o.y} y2={o.y + o.h} x1={o.x + o.w / 2} x2={o.x + o.w / 2} stroke={skin.window} stroke-width="1.6" vector-effect="non-scaling-stroke" />
            {/if}
          {:else if o.kind === 'door'}
            {#if along}
              <line x1={o.x} x2={o.x + o.w} y1={o.y + o.h / 2} y2={o.y + o.h / 2} stroke={skin.door} stroke-width="1" stroke-dasharray="4 3" vector-effect="non-scaling-stroke" />
            {:else}
              <line y1={o.y} y2={o.y + o.h} x1={o.x + o.w / 2} x2={o.x + o.w / 2} stroke={skin.door} stroke-width="1" stroke-dasharray="4 3" vector-effect="non-scaling-stroke" />
            {/if}
          {/if}
        </g>
      {/each}
      </g>

      <!-- names -->
      {#each floor.rooms ?? [] as r (r.id)}
        {@const [lx, ly] = labelAt(r.poly)}
        {@const m2 = area(r.poly)}
        {#if (r.kind !== 'void' && r.kind !== 'deck') || cam.k > fitK * 2}
          <text class="label" x={lx} y={ly} font-size={fs} fill={skin.label} text-anchor="middle">{label(r.name)}</text>
          {#if m2 >= 2 && cam.k > fitK * 1.2}
            <text class="label" x={lx} y={ly + fs * 1.25} font-size={fs * 0.85} fill={skin.sub} text-anchor="middle">{m2.toLocaleString(locale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 })} m²</text>
          {/if}
        {/if}
      {/each}

      {#if editing}
        {@render editLayer()}
      {/if}
    </svg>

    <!-- the devices: always the same size on screen -->
    {#each floor.devices ?? [] as spot (spot.id)}
      {@const d = hub.devices[spot.id]}
      {#if d && keep(d) && visible(spot.x, spot.y)}
        <div class="pin" style="left:{(spot.x - cam.x) * cam.k}px; top:{(spot.y - cam.y) * cam.k}px; --s:{pinScale}" class:named={cam.k > fitK * 1.7} class:asleep={editing && (layer !== 'items' || selection?.type === 'item')}>
          {@render token(spot, d)}
        </div>
      {/if}
    {/each}
  {/if}

  {#if W && H}
    <!-- each room's temperature, at its foot -->
    {#each climates as c (c.id)}
      {#if (c.w * cam.k >= 56 || editing) && visible(c.x, c.y)}
        <div class="climate {tone(c.t)}" style="left:{(c.x - cam.x) * cam.k}px; top:{(c.y - cam.y) * cam.k}px; --m:{sizes.gauges ?? 1}">
          <b>{num(c.t, 1)}°</b>
          <i class="gauge"><i style="left:{gaugeAt(c.t)}%"></i></i>
        </div>
      {/if}
    {/each}
    {#if floorTemp != null}
      <div class="climate floor-temp {tone(floorTemp)}" style="--m:{sizes.gauges ?? 1}">
        <span>{floor.name}</span>
        <b>{num(floorTemp, 1)}°</b>
        <i class="gauge"><i style="left:{gaugeAt(floorTemp)}%"></i></i>
      </div>
    {/if}
  {/if}

  {#if !fill}
    <div class="zoom" onpointerdown={(e) => e.stopPropagation()}>
      <button onclick={() => zoom(1.6)} aria-label={t('maison.plan.zoom_avant')}>+</button>
      <button onclick={() => zoom(1 / 1.6)} aria-label={t('maison.plan.zoom_arriere')}>−</button>
      <button onclick={fit} aria-label={t('maison.plan.tout_le_plan')}>⤢</button>
    </div>
  {/if}
</div>

<style>
  .drawing.fill {
    height: 100%;
    max-height: none;
    aspect-ratio: auto !important;
  }

  /* A room's temperature: a pill at its foot, a cold-to-hot gauge. */
  .climate {
    position: absolute;
    transform: translate(-50%, calc(-100% - 6px));
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--surface) 90%, transparent);
    box-shadow: 0 1px 6px rgb(0 0 0 / 14%);
    font-size: calc(12px * var(--m, 1));
    line-height: 1.2;
    pointer-events: none;
    white-space: nowrap;
    z-index: 1;
  }

  .climate .gauge {
    width: calc(34px * var(--m, 1));
  }

  .climate b {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .climate span {
    color: var(--ink-3);
    font-weight: 600;
  }

  .floor-temp {
    left: 12px;
    top: 12px;
    transform: none;
  }

  .gauge {
    position: relative;
    width: 34px;
    height: 5px;
    border-radius: 3px;
    background: linear-gradient(90deg, #4f8cff 0%, #3fc1d9 25%, #4cc26a 40%, #4cc26a 62%, #ffb340 80%, #ff5a3c 100%);
  }

  .gauge i {
    position: absolute;
    top: -3px;
    width: 3px;
    height: 11px;
    border-radius: 2px;
    background: var(--ink);
    box-shadow: 0 0 0 1.5px var(--surface);
    transform: translateX(-50%);
  }

  .cold b {
    color: #3b7bea;
  }

  .cool b {
    color: #2aa5bd;
  }

  .good b {
    color: #2f9e55;
  }

  .warm b {
    color: #e08a00;
  }

  .hot b {
    color: #e5482b;
  }

  .drawing {
    position: relative;
    width: 100%;
    max-height: max(320px, calc(100dvh - 360px));
    overflow: hidden;
    border-radius: var(--r-lg);
    box-shadow: var(--shadow);
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }

  .drawing.picking {
    cursor: crosshair;
    outline: 3px dashed var(--warm);
    outline-offset: -3px;
  }

  svg {
    display: block;
  }

  .label {
    font-family: inherit;
    font-weight: 650;
    pointer-events: none;
    paint-order: stroke;
  }

  .glow {
    pointer-events: none;
    mix-blend-mode: screen;
    transition: opacity 0.6s;
  }

  .warm {
    pointer-events: none;
    transition: opacity 0.6s;
  }

  .item {
    cursor: default;
  }

  :global(.editing) .item {
    cursor: grab;
  }

  .handle {
    cursor: grab;
  }

  .item .handle {
    cursor: nwse-resize;
  }

  .hit {
    fill: transparent;
    cursor: pointer;
  }

  .texture {
    pointer-events: none;
  }

  /* Edit: only the layer being arranged answers the finger. */
  .asleep {
    pointer-events: none;
  }

  .pin.asleep {
    opacity: 0.3;
  }

  .pin.asleep :global(.name),
  .pin.asleep :global(.unplace) {
    display: none;
  }

  .pin {
    position: absolute;
    display: grid;
    justify-items: center;
    transform: translate(-50%, -50%) scale(var(--s, 1));
  }

  .pin:not(.named) :global(.name) {
    display: none;
  }

  .zoom {
    position: absolute;
    right: 10px;
    bottom: 10px;
    display: grid;
    gap: 6px;
  }

  .zoom button {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: none;
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow-lift);
    font: inherit;
    font-size: 18px;
    font-weight: 700;
    cursor: pointer;
  }
</style>
