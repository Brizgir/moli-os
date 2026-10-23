<script>
  import { hub, home, value, act, note, nameOf, num } from '../lib/home.svelte.js';
  import { t, locale } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import HeatingCurves from '../ui/HeatingCurves.svelte';
  import { DAY, WEEK, STEP, stretches, moveBoundary, paint, copyDay, clock, nudge, kindKey, zoneColor, draftOf, normalize } from '../lib/heating.js';

  /** The week of heating, as Netatmo keeps it, in the family's words: the
   *  « réglages » (comfort, night…) with their temperature in each room, and
   *  which one holds when. Drag a boundary to move it, touch a stretch to
   *  change it, duplicate a schedule to try a new one. Saving asks for the
   *  code: a schedule is a person's, never an agent's. */
  let { route = '' } = $props();

  // The Netatmo homes that have schedules (the home device's service).
  const homes = $derived(Object.values(hub.devices ?? {}).filter((d) => d.service && d.manufacturer === 'Netatmo' && d.points?.some((p) => p.key === 'schedule')));
  const wanted = $derived(decodeURIComponent(route.split('/')[1] ?? ''));
  const homeId = $derived((homes.find((d) => d.id === wanted) ?? homes[0])?.id ?? null);

  let data = $state(null);
  let loading = $state(false);
  let error = $state('');
  let busy = $state(false);
  let chosen = $state(null);
  let draft = $state(null);
  /** `edit` an existing schedule, or `new`: a duplicate not yet created. */
  let mode = $state('edit');
  let original = $state('');
  let sheet = $state(null);
  let zoneOpen = $state(null);
  let copying = $state(null);

  const url = (name) => `/api/devices/${encodeURIComponent(homeId)}/service/${name}`;
  const headers = { 'content-type': 'application/json', 'x-moli-origin': 'ui' };

  async function load() {
    if (!homeId) return;
    loading = true;
    error = '';
    try {
      const res = await fetch(url('schedules'));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.error ?? res.statusText);
      take(body, chosen);
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  /** What Netatmo holds now; `select`: the schedule to show (else the active one). */
  function take(body, select) {
    data = body;
    const found = body.schedules.find((s) => s.id === select) ?? body.schedules.find((s) => s.id === body.active) ?? body.schedules[0];
    open(found);
  }

  function open(schedule) {
    sheet = null;
    zoneOpen = null;
    copying = null;
    mode = 'edit';
    chosen = schedule?.id ?? null;
    draft = schedule ? draftOf(schedule, data.rooms) : null;
    original = JSON.stringify(draft);
  }

  $effect(() => {
    if (homeId) load();
  });

  const dirty = $derived(draft != null && (mode === 'new' || JSON.stringify(draft) !== original));
  const zoneIndex = (id) => draft?.zones.findIndex((z) => z.id === id) ?? -1;
  const zoneOf = (id) => draft?.zones.find((z) => z.id === id);
  const colorOf = (id) => zoneColor(zoneOf(id), Math.max(0, zoneIndex(id)));
  const zoneName = (z) => z?.name || t(`maison.chauffage.reglage.${kindKey(z?.type)}`);
  const roomName = (id) => data?.rooms.find((r) => r.id === id)?.name ?? id;

  // Days by the house's language, Monday first (Netatmo's week).
  const days = $derived.by(() => {
    const fmt = new Intl.DateTimeFormat(locale(), { weekday: 'short' });
    const long = new Intl.DateTimeFormat(locale(), { weekday: 'long' });
    // 2024-01-01 was a Monday.
    return Array.from({ length: 7 }, (_, i) => {
      const d = new Date(Date.UTC(2024, 0, 1 + i, 12));
      return { short: fmt.format(d), long: long.format(d) };
    });
  });

  // ---- what overrides the schedule now ----------------------------------------------------

  const rooms = $derived(
    Object.values(hub.devices ?? {}).filter((d) => homeId && d.instance === hub.devices[homeId]?.instance && d.points?.some((p) => p.key === 'setpoint')),
  );
  const manual = $derived(rooms.filter((d) => value(d.id, 'setpoint_mode') === 'manual'));
  const homeMode = $derived(homeId ? value(homeId, 'mode') : null);
  const until = (ms) => (typeof ms === 'number' && ms > 0 ? new Date(ms).toLocaleString(locale(), { weekday: 'short', hour: 'numeric', minute: '2-digit' }) : null);

  // ---- the week ---------------------------------------------------------------------------

  /** Where the finger is on a day's bar, in minutes of the week. */
  function minuteAt(e, bar, day) {
    const box = bar.getBoundingClientRect();
    const x = Math.min(1, Math.max(0, (e.clientX - box.left) / box.width));
    return day * DAY + x * DAY;
  }

  let dragging = $state(null);

  function grab(e, slot, day) {
    e.preventDefault();
    e.stopPropagation();
    const bar = e.currentTarget.closest('.bar');
    e.currentTarget.setPointerCapture?.(e.pointerId);
    dragging = { slot, day, bar };
  }

  function drag(e) {
    if (!dragging) return;
    draft.timetable = moveBoundary(draft.timetable, dragging.slot, minuteAt(e, dragging.bar, dragging.day));
  }

  function drop() {
    if (!dragging) return;
    dragging = null;
    draft.timetable = normalize(draft.timetable);
  }

  /** Nudges a boundary by a quarter hour (keyboard, and the sheet). */
  function step(slot, delta) {
    draft.timetable = normalize(moveBoundary(draft.timetable, slot, draft.timetable[slot].start + delta));
  }

  function touch(s) {
    zoneOpen = null;
    copying = null;
    sheet = { from: s.from, to: s.to, zone: s.zone };
  }

  function apply() {
    draft.timetable = paint(draft.timetable, sheet.from, sheet.to, sheet.zone);
    sheet = null;
  }

  const moveEdge = (edge, delta) => {
    const next = Math.min(WEEK, Math.max(0, sheet[edge] + delta));
    if (edge === 'from' && next < sheet.to) sheet.from = next;
    if (edge === 'to' && next > sheet.from) sheet.to = next;
  };

  function copyTo(day) {
    copying = { day, targets: [] };
    sheet = null;
    zoneOpen = null;
  }

  function doCopy() {
    draft.timetable = copyDay(draft.timetable, copying.day, copying.targets);
    copying = null;
  }

  // ---- zones and temperatures ---------------------------------------------------------------

  function setTemp(zone, room, delta) {
    const r = zone.rooms.find((x) => x.id === room);
    if (r) r.temp = nudge(r.temp, delta);
  }

  // ---- saving: the code first, then Netatmo, then what it holds -----------------------------

  async function send(action, body, label, select) {
    busy = true;
    error = '';
    try {
      const res = await fetch(url(action), { method: 'POST', headers, body: JSON.stringify(body) });
      if (res.status === 403) {
        home.held = { label, reason: 'ton code', custom: () => send(action, body, label, select) };
        return;
      }
      const answer = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(answer.error ?? res.statusText);
      const known = new Set((data?.schedules ?? []).map((s) => s.id));
      const fresh = select === 'new' ? answer.schedules.find((s) => !known.has(s.id))?.id : select;
      take(answer, fresh);
      note(t('maison.chauffage.enregistre'), 'info');
    } catch (e) {
      error = e.message;
    } finally {
      busy = false;
    }
  }

  function save() {
    const body = { ...$state.snapshot(draft) };
    if (mode === 'new') {
      delete body.id;
      send('create', body, t('maison.chauffage.creer_titre', { name: draft.name }), 'new');
    } else {
      send('sync', body, t('maison.chauffage.enregistrer_titre', { name: draft.name }), draft.id);
    }
  }

  function activate() {
    send('activate', { id: draft.id }, t('maison.chauffage.activer_titre', { name: draft.name }), draft.id);
  }

  function duplicate() {
    const copy = $state.snapshot(draft);
    const base = t('maison.chauffage.copie', { name: copy.name });
    draft = { ...copy, id: null, name: base.slice(0, 60) };
    mode = 'new';
    chosen = null;
    sheet = null;
  }

  function discard() {
    open(data.schedules.find((s) => s.id === chosen) ?? data.schedules.find((s) => s.id === data.active));
  }

  const hours = [0, 6, 12, 18, 24];
  const pct = (minutes) => `${(minutes / DAY) * 100}%`;
</script>

<div class="chauffage">
  <header>
    <a class="back" href="#/pieces" aria-label={t('maison.chauffage.retour')}><Icon name="back" size={20} /></a>
    <div>
      <h1 class="page-title">{t('maison.chauffage.titre')}</h1>
      <p class="page-sub">{data?.home?.name ?? ''}</p>
    </div>
  </header>

  {#if !homeId}
    <p class="card muted">{t('maison.chauffage.aucune_maison')}</p>
  {:else if loading && !data}
    <p class="card muted">{t('maison.chauffage.lecture')}</p>
  {:else if !data}
    <div class="card">
      <p class="error">{error || t('maison.chauffage.illisible')}</p>
      <button class="btn" onclick={load}>{t('maison.chauffage.reessayer')}</button>
    </div>
  {:else}
    <!-- What holds over the schedule right now. -->
    {#if homeMode === 'away' || homeMode === 'hg'}
      <p class="banner cool"><Icon name={homeMode === 'hg' ? 'snowflake' : 'home'} size={18} />{t(homeMode === 'hg' ? 'maison.chauffage.maison_hors_gel' : 'maison.chauffage.maison_absente')}</p>
    {/if}
    {#each manual as d (d.id)}
      {@const end = until(value(d.id, 'setpoint_end'))}
      <div class="banner warm">
        <Icon name="hand" size={18} />
        <span>{end ? t('maison.chauffage.manuel_jusqua', { room: nameOf(d.id), temp: num(value(d.id, 'setpoint'), 1), until: end }) : t('maison.chauffage.manuel', { room: nameOf(d.id), temp: num(value(d.id, 'setpoint'), 1) })}</span>
        <button class="btn small" onclick={() => act(`${d.id}/setpoint_mode`, 'schedule', t('maison.chauffage.revenir_titre', { room: nameOf(d.id) }))}>{t('maison.chauffage.revenir')}</button>
      </div>
    {/each}

    <!-- The schedules: one is followed. -->
    <section class="card">
      <h2>{t('maison.chauffage.plannings')}</h2>
      <div class="choices" role="radiogroup" aria-label={t('maison.chauffage.plannings')}>
        {#each data.schedules as s (s.id)}
          <button class="choice" class:on={mode === 'edit' && chosen === s.id} role="radio" aria-checked={mode === 'edit' && chosen === s.id} disabled={busy} onclick={() => open(s)}>
            <b>{s.name}</b>
            {#if s.id === data.active}<small>{t('maison.chauffage.en_cours')}</small>{/if}
          </button>
        {/each}
        {#if mode === 'new'}
          <span class="choice on"><b>{draft.name}</b><small>{t('maison.chauffage.nouveau')}</small></span>
        {/if}
      </div>
      {#if draft}
        <div class="actions">
          {#if mode === 'edit' && draft.id !== data.active}
            <button class="btn primary" disabled={busy || dirty} onclick={activate}><Icon name="ok" size={18} />{t('maison.chauffage.utiliser')}</button>
          {/if}
          {#if mode === 'edit'}
            <button class="btn" disabled={busy} onclick={duplicate}><Icon name="copy" size={18} />{t('maison.chauffage.dupliquer')}</button>
          {/if}
        </div>
        {#if mode === 'edit' && draft.id !== data.active && dirty}
          <small class="muted">{t('maison.chauffage.enregistrer_avant')}</small>
        {/if}
        <label class="name">
          {t('maison.chauffage.nom')}
          <input bind:value={draft.name} maxlength="60" />
        </label>
      {/if}
    </section>

    {#if draft}
      <!-- The settings: comfort, night… and their temperatures. -->
      <section class="card">
        <h2>{t('maison.chauffage.reglages')}</h2>
        <p class="muted">{t('maison.chauffage.reglages_aide')}</p>
        <div class="zones">
          {#each draft.zones as z, i (z.id)}
            <button class="zone" class:open={zoneOpen === z.id} style:--zone={zoneColor(z, i)} onclick={() => ((zoneOpen = zoneOpen === z.id ? null : z.id), (sheet = null), (copying = null))} aria-expanded={zoneOpen === z.id}>
              <span class="dot"></span>
              <b>{zoneName(z)}</b>
              <small>{z.rooms.map((r) => `${num(r.temp, 1)}°`).join(' · ')}</small>
            </button>
          {/each}
        </div>
        {#each draft.zones as z, i (z.id)}
          {#if zoneOpen === z.id}
            <div class="zone-edit" style:--zone={zoneColor(z, i)}>
              {#each z.rooms as r (r.id)}
                <div class="temp-row">
                  <span>{roomName(r.id)}</span>
                  <button class="round" aria-label={t('maison.chauffage.moins')} onclick={() => setTemp(z, r.id, -0.5)}><Icon name="minus" size={20} /></button>
                  <b class="num">{num(r.temp, 1)}°</b>
                  <button class="round" aria-label={t('maison.chauffage.plus')} onclick={() => setTemp(z, r.id, 0.5)}><Icon name="plus" size={20} /></button>
                </div>
              {/each}
            </div>
          {/if}
        {/each}
        <h3>{t('maison.chauffage.maison_vide')}</h3>
        <div class="temp-row">
          <span>{t('maison.chauffage.absence')}</span>
          <button class="round" aria-label={t('maison.chauffage.moins')} onclick={() => (draft.away_temp = nudge(draft.away_temp, -0.5))}><Icon name="minus" size={20} /></button>
          <b class="num">{num(draft.away_temp, 1)}°</b>
          <button class="round" aria-label={t('maison.chauffage.plus')} onclick={() => (draft.away_temp = nudge(draft.away_temp, 0.5))}><Icon name="plus" size={20} /></button>
        </div>
        <div class="temp-row">
          <span>{t('maison.chauffage.hors_gel')}</span>
          <button class="round" aria-label={t('maison.chauffage.moins')} onclick={() => (draft.hg_temp = nudge(draft.hg_temp, -0.5))}><Icon name="minus" size={20} /></button>
          <b class="num">{num(draft.hg_temp, 1)}°</b>
          <button class="round" aria-label={t('maison.chauffage.plus')} onclick={() => (draft.hg_temp = nudge(draft.hg_temp, 0.5))}><Icon name="plus" size={20} /></button>
        </div>
      </section>

      <!-- The week. -->
      <section class="card week">
        <h2>{t('maison.chauffage.semaine')}</h2>
        <p class="muted">{t('maison.chauffage.semaine_aide')}</p>
        <div class="hours" aria-hidden="true">
          {#each hours as h (h)}<span style:left={pct(h * 60)}>{h}h</span>{/each}
        </div>
        {#each days as d, day (day)}
          <div class="day">
            <button class="day-name" onclick={() => copyTo(day)} title={t('maison.chauffage.copier_titre', { day: d.long })}>{d.short}</button>
            <div class="bar" onpointermove={drag} onpointerup={drop} onpointercancel={drop} role="group" aria-label={d.long}>
              {#each stretches(draft.timetable, day) as s (s.from)}
                {@const z = zoneOf(s.zone)}
                <button
                  class="stretch"
                  class:picked={sheet && sheet.from <= s.from && sheet.to >= s.to}
                  style:left={pct(s.from - day * DAY)}
                  style:width={pct(s.to - s.from)}
                  style:--zone={colorOf(s.zone)}
                  onclick={() => touch(s)}
                  aria-label={t('maison.chauffage.creneau', { day: d.long, from: clock(s.from), to: clock(s.to, true), name: zoneName(z) })}
                >
                  {#if s.to - s.from >= 150}<span>{zoneName(z)}</span>{/if}
                </button>
                {#if s.slot > 0 && s.from > day * DAY}
                  <span
                    class="handle"
                    class:active={dragging?.slot === s.slot}
                    style:left={pct(s.from - day * DAY)}
                    role="slider"
                    tabindex="0"
                    aria-label={t('maison.chauffage.limite', { day: d.long })}
                    aria-valuemin="0"
                    aria-valuemax={DAY}
                    aria-valuenow={s.from - day * DAY}
                    aria-valuetext={clock(s.from)}
                    onpointerdown={(e) => grab(e, s.slot, day)}
                    onkeydown={(e) => {
                      if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') (e.preventDefault(), step(s.slot, -STEP));
                      if (e.key === 'ArrowRight' || e.key === 'ArrowUp') (e.preventDefault(), step(s.slot, STEP));
                    }}
                  ><i></i>{#if dragging?.slot === s.slot}<em>{clock(draft.timetable[s.slot]?.start ?? s.from)}</em>{/if}</span>
                {/if}
              {/each}
            </div>
          </div>
        {/each}

        {#if sheet}
          <div class="sheet">
            <div class="edges">
              <div class="edge">
                <small>{t('maison.chauffage.de')}</small>
                <div class="stepper">
                  <button class="round" aria-label={t('maison.chauffage.plus_tot')} onclick={() => moveEdge('from', -STEP)}><Icon name="minus" size={18} /></button>
                  <b class="num">{days[Math.min(6, Math.floor(sheet.from / DAY))].short} {clock(sheet.from)}</b>
                  <button class="round" aria-label={t('maison.chauffage.plus_tard')} onclick={() => moveEdge('from', STEP)}><Icon name="plus" size={18} /></button>
                </div>
              </div>
              <div class="edge">
                <small>{t('maison.chauffage.a')}</small>
                <div class="stepper">
                  <button class="round" aria-label={t('maison.chauffage.plus_tot')} onclick={() => moveEdge('to', -STEP)}><Icon name="minus" size={18} /></button>
                  <b class="num">{days[Math.min(6, Math.floor((sheet.to - 1) / DAY))].short} {clock(sheet.to, true)}</b>
                  <button class="round" aria-label={t('maison.chauffage.plus_tard')} onclick={() => moveEdge('to', STEP)}><Icon name="plus" size={18} /></button>
                </div>
              </div>
            </div>
            <div class="zones">
              {#each draft.zones as z, i (z.id)}
                <button class="zone" class:open={sheet.zone === z.id} style:--zone={zoneColor(z, i)} onclick={() => (sheet.zone = z.id)}>
                  <span class="dot"></span><b>{zoneName(z)}</b>
                </button>
              {/each}
            </div>
            <div class="actions">
              <button class="btn primary" onclick={apply}><Icon name="ok" size={18} />{t('maison.chauffage.appliquer')}</button>
              <button class="btn" onclick={() => (sheet = null)}>{t('maison.chauffage.fermer')}</button>
            </div>
          </div>
        {/if}

        {#if copying}
          <div class="sheet">
            <p>{t('maison.chauffage.copier_vers', { day: days[copying.day].long })}</p>
            <div class="choices">
              {#each days as d, day (day)}
                {#if day !== copying.day}
                  <label class="choice" class:on={copying.targets.includes(day)}>
                    <input type="checkbox" bind:group={copying.targets} value={day} />
                    <b>{d.short}</b>
                  </label>
                {/if}
              {/each}
            </div>
            <div class="actions">
              <button class="btn primary" disabled={!copying.targets.length} onclick={doCopy}><Icon name="copy" size={18} />{t('maison.chauffage.copier')}</button>
              <button class="btn" onclick={() => (copying = null)}>{t('maison.chauffage.fermer')}</button>
            </div>
          </div>
        {/if}
      </section>

      <p class="muted note">{t('maison.chauffage.manuel_prime')}</p>

      {#if error}<p class="error">{error}</p>{/if}

      {#if dirty}
        <div class="savebar">
          <button class="btn" disabled={busy} onclick={discard}>{t('maison.chauffage.annuler')}</button>
          <button class="btn primary" disabled={busy || !draft.name.trim()} onclick={save}>
            <Icon name="save" size={18} />{busy ? t('maison.chauffage.envoi') : mode === 'new' ? t('maison.chauffage.creer') : t('maison.chauffage.enregistrer')}
          </button>
          <small class="muted">{t('maison.chauffage.code_demande')}</small>
        </div>
      {/if}
    {/if}
  {/if}

  {#if homeId}
    <!-- How each room went: read, aimed at, heated. -->
    <HeatingCurves instance={hub.devices[homeId]?.instance ?? null} />
  {/if}
</div>

<style>
  .chauffage {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--sp-4);
    padding-bottom: 96px;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }

  .back {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-2);
    color: inherit;
    flex: none;
  }

  section {
    display: grid;
    gap: var(--sp-3);
  }

  h2,
  h3 {
    margin: 0;
    font-size: 1.05rem;
  }

  h3 {
    margin-top: 6px;
    font-size: 0.95rem;
  }

  p {
    margin: 0;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin: 0;
    padding: 12px 16px;
    border-radius: var(--r-md);
  }

  .banner span {
    flex: 1;
    min-width: 12em;
  }

  .banner.warm {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  .banner.cool {
    background: var(--cool-soft);
  }

  .choices,
  .zones,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }

  .choice,
  .zone {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-height: 48px;
    padding: 8px 16px;
    border: 2px solid transparent;
    border-radius: 16px;
    background: var(--surface-2);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .choice small,
  .zone small {
    color: var(--ink-3);
  }

  .choice.on {
    border-color: var(--warm);
    background: var(--warm-soft);
  }

  .choice input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .zone .dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--zone);
    flex: none;
  }

  .zone.open {
    border-color: var(--zone);
  }

  .zone-edit {
    display: grid;
    gap: 6px;
    padding: var(--sp-3);
    border-radius: 16px;
    background: color-mix(in srgb, var(--zone) 12%, transparent);
  }

  .temp-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 48px 4.5em 48px;
    align-items: center;
    gap: var(--sp-2);
  }

  .temp-row .num {
    text-align: center;
    font-size: 1.15rem;
  }

  .round {
    width: 48px;
    height: 48px;
    border: 0;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface);
    color: inherit;
    cursor: pointer;
    box-shadow: 0 1px 2px rgb(0 0 0 / 12%);
  }

  .name {
    display: grid;
    gap: var(--sp-1);
  }

  .name input {
    font: inherit;
    padding: 10px 12px;
    border-radius: 12px;
    border: 1px solid var(--line, rgb(0 0 0 / 15%));
    background: var(--surface);
    color: inherit;
    min-width: 0;
  }

  .week .hours {
    position: relative;
    height: 16px;
    margin-left: 56px;
    font-size: 0.75rem;
    color: var(--ink-3);
  }

  .hours span {
    position: absolute;
    transform: translateX(-50%);
  }

  .hours span:first-child {
    transform: none;
  }

  .hours span:last-child {
    transform: translateX(-100%);
  }

  .day {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr);
    align-items: center;
    gap: var(--sp-2);
  }

  .day-name {
    height: 52px;
    border: 0;
    border-radius: 12px;
    background: var(--surface-2);
    color: inherit;
    font: inherit;
    font-weight: 650;
    text-transform: capitalize;
    cursor: pointer;
  }

  .bar {
    position: relative;
    height: 52px;
    border-radius: 12px;
    overflow: visible;
    background: var(--surface-3);
    touch-action: none;
  }

  .stretch {
    position: absolute;
    top: 0;
    bottom: 0;
    border: 0;
    padding: 0 6px;
    background: var(--zone);
    color: #fff;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 650;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
  }

  .stretch:first-child {
    border-radius: 12px 0 0 12px;
  }

  .stretch:last-of-type {
    border-radius: 0 12px 12px 0;
  }

  .stretch.picked {
    outline: 3px solid var(--ink, #222);
    outline-offset: -3px;
    z-index: 1;
  }

  .handle {
    position: absolute;
    top: -6px;
    bottom: -6px;
    width: 32px;
    transform: translateX(-50%);
    display: grid;
    place-items: center;
    cursor: ew-resize;
    touch-action: none;
    z-index: 2;
  }

  .handle i {
    width: 6px;
    height: 100%;
    border-radius: 3px;
    background: var(--surface);
    box-shadow: 0 0 0 1px rgb(0 0 0 / 25%);
  }

  .handle.active i,
  .handle:focus-visible i {
    background: var(--ink, #222);
  }

  .handle em {
    position: absolute;
    top: -28px;
    padding: 2px 8px;
    border-radius: 8px;
    background: var(--ink, #222);
    color: var(--surface);
    font-style: normal;
    font-size: 0.8rem;
    white-space: nowrap;
  }

  .sheet {
    display: grid;
    gap: var(--sp-3);
    padding: 14px;
    border-radius: 18px;
    background: var(--surface-2);
  }

  .edges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-3);
  }

  .edge {
    display: grid;
    gap: var(--sp-1);
  }

  .stepper {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .stepper .num {
    min-width: 6.5em;
    text-align: center;
    text-transform: capitalize;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 48px;
    border: 0;
    border-radius: 999px;
    padding: 10px 18px;
    font: inherit;
    font-weight: 650;
    background: var(--surface-2);
    color: var(--ink);
    cursor: pointer;
  }

  .btn.small {
    min-height: 40px;
    padding: 6px 14px;
    background: var(--surface);
  }

  .btn.primary {
    background: var(--warm);
    color: #fff;
  }

  .btn:disabled,
  .choice:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .savebar {
    position: sticky;
    bottom: 12px;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-start;
    gap: 10px;
    /* Room on the right for the « Ask Moli » bubble. */
    padding: 12px 16px;
    margin-right: 200px;
    border-radius: var(--r-md);
    background: var(--surface);
    box-shadow: 0 4px 18px rgb(0 0 0 / 14%);
    z-index: 3;
  }

  .savebar small {
    flex-basis: 100%;
  }

  @media (max-width: 767.98px) {
    .savebar {
      margin-right: 0;
      bottom: 84px;
    }
  }

  .note {
    font-size: 0.9rem;
  }

  .error {
    color: var(--danger, #b3261e);
  }

  button:focus-visible,
  .handle:focus-visible,
  input:focus-visible {
    outline: 2px solid currentColor;
    outline-offset: 2px;
  }
</style>
