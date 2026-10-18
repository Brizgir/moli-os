<script>
  import { onMount } from 'svelte';
  import { hub, home, value, act, nameOf, roomOf, protectedRoom, num, deg, clock, hidden, heatedRoom, deviceKind, pending } from '../lib/home.svelte.js';
  import { timed, useTimed } from '../lib/timed.svelte.js';
  import { nextHeating } from '../lib/today.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** The heating at a glance (Netatmo): each heated room with its
   *  temperature and its setpoint, the next change the timed automations
   *  make, the week's schedule, and « Mode absence » for the whole home. */
  onMount(useTimed);

  const order = $derived((home.config?.rooms ?? []).map((r) => r.name));
  const rooms = $derived(
    Object.values(hub.devices)
      .filter((d) => !hidden(d.id) && heatedRoom(d))
      .map((d) => ({ id: d.id, name: roomOf(d) ?? nameOf(d.id) }))
      .sort((a, b) => {
        const i = (n) => (order.includes(n) ? order.indexOf(n) : order.length);
        return i(a.name) - i(b.name) || a.name.localeCompare(b.name);
      }),
  );

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
</script>

<section class="card heating">
  <div class="card-head">
    <h2><Icon name="radiator" size={18} />{t('maison.accueil.chauffage.titre')}</h2>
    {#if away}
      <span class="chip cool">{t('maison.accueil.chauffage.en_absence')}</span>
    {:else if mode === 'hg'}
      <span class="chip cool">{t('maison.accueil.chauffage.maison_hors_gel')}</span>
    {:else if nextText}
      <span class="next">{nextText}</span>
    {/if}
  </div>
  <div class="rooms">
    {#each rooms as r (r.id)}
      <div class="room" class:warm={value(r.id, 'heating') === true}>
        <span class="name">
          {r.name}
          {#if protectedRoom(r.name)}<span class="lock" role="img" aria-label={t('commun.piece.protegee_titre')}><Icon name="lock" size={13} /></span>{/if}
        </span>
        <b class="num">{value(r.id, 'temperature') != null ? `${num(value(r.id, 'temperature'), 1)}°` : '—'}</b>
        <small>{aim(r.id)}</small>
      </div>
    {/each}
  </div>
  <div class="actions">
    <a class="primary" href="#/chauffage">{t('maison.accueil.chauffage.voir_planning')}</a>
    {#if homeDevice}
      <button class="secondary" class:on={away} aria-pressed={away} disabled={pending(homeDevice.id, 'mode')} onclick={toggleAway}>
        {away ? t('maison.accueil.chauffage.retour_planning') : t('maison.accueil.chauffage.mode_absence')}
      </button>
    {/if}
  </div>
</section>

<style>
  .heating {
    display: grid;
    gap: 16px;
  }

  .card-head {
    flex-wrap: wrap;
    gap: 8px 12px;
  }

  .next {
    font-size: 14px;
    color: var(--ink-2);
  }

  .rooms {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 12px;
  }

  .room {
    display: grid;
    gap: 4px;
    padding: 14px 16px;
    border-radius: 18px;
    background: var(--surface-2);
    min-width: 0;
  }

  .room.warm {
    background: linear-gradient(155deg, var(--warm-soft) 0%, var(--surface-2) 85%);
  }

  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
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
    font-size: 13px;
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
