<script>
  import { onMount } from 'svelte';
  import { hub, home, value, act, nameOf, roomOf, protectedRoom, num, deg, clock, heatedRoom, heatedRooms, deviceKind, pending } from '../lib/home.svelte.js';
  import { timed, useTimed } from '../lib/timed.svelte.js';
  import { nextHeating } from '../lib/today.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** The heating at a glance (Netatmo): each heated room with its
   *  temperature and its setpoint, the next change the timed automations
   *  make, the week's schedule, and « Mode absence » for the whole home. */
  onMount(useTimed);

  /** From the home page's arrangement: `only` (the rooms to show, in that
   *  order; null: all of them, the layout's order), what to show (`next`
   *  change, the `planning` link, the `away` button), `title`. */
  let { only = null, next: showNext = true, planning = true, away: showAway = true, title = null } = $props();

  const rooms = $derived(heatedRooms(only));

  // The home the rooms belong to: its mode (`schedule`, `away`, `hg`).
  const homeDevice = $derived.by(() => {
    const instance = hub.devices[rooms[0]?.id]?.instance;
    return (
      Object.values(hub.devices).find(
        (d) => d.instance === instance && d.points.some((p) => p.key === 'mode' && p.access?.write && p.kind?.values?.includes('away')),
      ) ?? null
    );
  });
  const mode = $derived(homeDevice ? value(homeDevice.id, 'mode') : null);
  const away = $derived(mode === 'away');

  function toggleAway() {
    act(`${homeDevice.id}/mode`, away ? 'schedule' : 'away', t(away ? 'maison.accueil.chauffage.retour_planning' : 'maison.accueil.chauffage.mode_absence'));
  }

  /** A point the heating answers to: a heated room, its home, a clim. */
  function heating(point) {
    const d = hub.devices[point.slice(0, point.lastIndexOf('/'))];
    return !!d && (heatedRoom(d) || d.id === homeDevice?.id || deviceKind(d) === 'climate');
  }
  const next = $derived(nextHeating(timed.list, new Date(home.now), heating));
  const nextText = $derived.by(() => {
    if (!next) return '';
    const at = new Date(home.now);
    at.setHours(Math.floor(next.minutes / 60), next.minutes % 60, 0, 0);
    return t(next.tomorrow ? 'maison.accueil.chauffage.prochain_demain' : 'maison.accueil.chauffage.prochain', { name: next.name, time: clock(at) });
  });

  /** What a room aims at, in words. */
  function aim(id) {
    const sp = value(id, 'setpoint');
    const m = value(id, 'setpoint_mode');
    if (m === 'off') return t('maison.accueil.chauffage.arrete');
    if (m === 'hg') return t('maison.accueil.chauffage.hors_gel');
    if (sp == null) return '';
    const temp = deg(sp);
    if (m === 'manual') {
      const end = value(id, 'setpoint_end');
      return typeof end === 'number' && end > 0
        ? t('maison.accueil.chauffage.consigne_jusqua', { temp, time: clock(end) })
        : t('maison.accueil.chauffage.consigne_permanente', { temp });
    }
    return t('maison.accueil.chauffage.consigne', { temp });
  }

  // ---- a manual setpoint, from a room's tile ------------------------------------------
  // Touching a room opens its setpoint; − / + choose, « Appliquer » sends it.
  // Netatmo keeps it manual for a while, then the room goes back to its schedule.

  const MIN = 7;
  const MAX = 30;
  const STEP = 0.5;
  let picked = $state(null);
  let wanted = $state(null);

  const writable = (id) => hub.devices[id]?.points.some((p) => p.key === 'setpoint' && p.access?.write);
  const current = $derived(picked ? value(picked, 'setpoint') : null);
  const shown = $derived(wanted ?? (typeof current === 'number' ? current : 19));
  const pickedName = $derived(rooms.find((r) => r.id === picked)?.name ?? '');

  function pick(id) {
    picked = picked === id ? null : id;
    wanted = null;
  }

  function nudge(delta) {
    wanted = Math.min(MAX, Math.max(MIN, Math.round((shown + delta) / STEP) * STEP));
  }

  async function apply() {
    const id = picked;
    const temp = shown;
    const done = await act(`${id}/setpoint`, temp, t('maison.accueil.chauffage.consigne_titre', { room: pickedName, temp: deg(temp) }));
    if (done !== 'error' && picked === id) wanted = null;
  }

  function backToSchedule() {
    act(`${picked}/setpoint_mode`, 'schedule', t('maison.chauffage.revenir_titre', { room: pickedName }));
    wanted = null;
  }
</script>

{#if rooms.length}
<section class="card heating">
  <div class="card-head">
    <h2><Icon name="radiator" size={18} />{title ?? t('maison.accueil.chauffage.titre')}</h2>
    {#if away}
      <span class="chip cool">{t('maison.accueil.chauffage.en_absence')}</span>
    {:else if mode === 'hg'}
      <span class="chip cool">{t('maison.accueil.chauffage.maison_hors_gel')}</span>
    {:else if nextText && showNext}
      <span class="next">{nextText}</span>
    {/if}
  </div>
  <div class="rooms">
    {#each rooms as r (r.id)}
      <svelte:element
        this={writable(r.id) ? 'button' : 'div'}
        class="room"
        class:warm={value(r.id, 'heating') === true}
        class:picked={picked === r.id}
        class:manual={value(r.id, 'setpoint_mode') === 'manual'}
        aria-expanded={writable(r.id) ? picked === r.id : undefined}
        aria-label={writable(r.id) ? t('maison.accueil.chauffage.regler', { room: r.name }) : undefined}
        onclick={writable(r.id) ? () => pick(r.id) : undefined}
        role={writable(r.id) ? undefined : 'group'}
      >
        <span class="name">
          {r.name}
          {#if value(r.id, 'setpoint_mode') === 'manual'}<span class="hand"><Icon name="hand" size={13} /></span>{/if}
          {#if protectedRoom(r.name)}<span class="lock" role="img" aria-label={t('commun.piece.protegee_titre')}><Icon name="lock" size={13} /></span>{/if}
        </span>
        <b class="num">{value(r.id, 'temperature') != null ? `${num(value(r.id, 'temperature'), 1)}°` : '—'}</b>
        <small>{aim(r.id)}</small>
      </svelte:element>
    {/each}
  </div>
  {#if picked}
    {@const isManual = value(picked, 'setpoint_mode') === 'manual'}
    {@const sending = pending(picked, 'setpoint')}
    <div class="setter" role="group" aria-label={t('maison.accueil.chauffage.regler', { room: pickedName })}>
      <div class="setter-head">
        <div>
          <b>{pickedName}</b>
          <small>{aim(picked) || '—'}</small>
        </div>
        <button class="close" aria-label={t('maison.chauffage.fermer')} onclick={() => (picked = null)}><Icon name="close" size={18} /></button>
      </div>
      <div class="stepper">
        <button class="round" aria-label={t('maison.chauffage.moins')} disabled={shown <= MIN} onclick={() => nudge(-STEP)}><Icon name="minus" size={22} /></button>
        <output class="num" aria-live="polite">{num(shown, 1)}°</output>
        <button class="round" aria-label={t('maison.chauffage.plus')} disabled={shown >= MAX} onclick={() => nudge(STEP)}><Icon name="plus" size={22} /></button>
      </div>
      <p class="hint">{t('maison.accueil.chauffage.manuel_aide')}</p>
      <div class="actions">
        <button class="primary" disabled={sending || (wanted == null && isManual)} onclick={apply}>
          {sending ? t('maison.accueil.chauffage.envoi') : t('maison.accueil.chauffage.appliquer', { temp: num(shown, 1) })}
        </button>
        {#if isManual}
          <button class="secondary" disabled={pending(picked, 'setpoint_mode')} onclick={backToSchedule}>{t('maison.accueil.chauffage.retour_planning')}</button>
        {/if}
      </div>
    </div>
  {/if}
  {#if planning || (homeDevice && showAway)}
    <div class="actions">
      {#if planning}<a class="primary" href="#/chauffage">{t('maison.accueil.chauffage.voir_planning')}</a>{/if}
      {#if homeDevice && showAway}
        <button class="secondary" class:on={away} aria-pressed={away} disabled={pending(homeDevice.id, 'mode')} onclick={toggleAway}>
          {away ? t('maison.accueil.chauffage.retour_planning') : t('maison.accueil.chauffage.mode_absence')}
        </button>
      {/if}
    </div>
  {/if}
</section>
{/if}

<style>
  .heating {
    display: grid;
    gap: var(--sp-4);
  }

  .card-head {
    flex-wrap: wrap;
    gap: 8px 12px;
  }

  .next {
    font-size: var(--fs-md);
    color: var(--ink-2);
  }

  .rooms {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: var(--sp-3);
  }

  .room {
    display: grid;
    gap: var(--sp-1);
    padding: 14px 16px;
    border-radius: 18px;
    background: var(--surface-2);
    min-width: 0;
  }

  button.room {
    border: 2px solid transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color 0.15s;
  }

  button.room:hover {
    border-color: var(--line);
  }

  .room.warm {
    background: linear-gradient(155deg, var(--warm-soft) 0%, var(--surface-2) 85%);
  }

  .room.picked,
  .room.picked:hover {
    border-color: var(--warm);
  }

  .hand {
    display: inline-grid;
    color: var(--warm);
  }

  .setter {
    display: grid;
    gap: 14px;
    padding: var(--sp-4);
    border-radius: 18px;
    background: var(--warm-soft);
  }

  .setter-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .setter-head div {
    display: grid;
    gap: 2px;
  }

  .setter-head small,
  .hint {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--ink-2);
  }

  .close,
  .round {
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: var(--surface);
    color: var(--ink);
    cursor: pointer;
    flex: none;
  }

  .close {
    width: 36px;
    height: 36px;
  }

  .stepper {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-5);
  }

  .round {
    width: 52px;
    height: 52px;
    box-shadow: 0 1px 2px rgb(0 0 0 / 12%);
  }

  .round:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .stepper output {
    min-width: 3.4em;
    text-align: center;
    font-size: 40px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  button.room:focus-visible,
  .round:focus-visible,
  .close:focus-visible {
    outline: 2px solid currentColor;
    outline-offset: 2px;
  }

  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-md);
    font-weight: 600;
    color: var(--ink-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lock {
    display: inline-grid;
    color: var(--cool);
  }

  .room b {
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .room small {
    font-size: var(--fs-sm);
    color: var(--ink-3);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .primary,
  .secondary {
    display: inline-flex;
    align-items: center;
    min-height: 44px;
    padding: 0 18px;
    border-radius: 999px;
    font: inherit;
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
  }

  .primary {
    background: var(--ink);
    color: var(--surface);
  }

  .secondary {
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
  }

  .secondary.on {
    border-color: transparent;
    background: var(--cool-soft);
    color: var(--cool);
  }

  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .secondary:disabled {
    opacity: 0.6;
    cursor: progress;
  }

  @media (max-width: 767.98px) {
    .rooms {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px;
    }

    .room b {
      font-size: 24px;
    }
  }
</style>
