<script>
  import { hub, home, value, act, hidden, relative, nameOf } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';
  import Tile from './Tile.svelte';

  /** The Ring location's watch (« Modes » in the Ring app): disarmed, home,
   *  away, with the current one in evidence. Changing it is a person's
   *  call: the server holds the order until the code is given (202), the
   *  PIN sheet opens on the press. `compact`: in a card of the home page. */
  let { compact = false } = $props();

  const MODES = [
    ['disarmed', 'power'],
    ['home', 'home'],
    ['away', 'shield'],
  ];
  const isWatch = (d) => d.manufacturer === 'Ring' && (d.points ?? []).some((p) => p.key === 'mode' && p.access?.write && p.kind?.values?.includes('away'));
  const watches = $derived(Object.values(hub.devices).filter((d) => !hidden(d.id) && isWatch(d)));
  const since = (id) => {
    const at = value(id, 'mode_since');
    const ms = typeof at === 'string' ? Date.parse(at) : NaN;
    return Number.isNaN(ms) ? null : ms;
  };

  async function choose(d, mode) {
    const point = `${d.id}/mode`;
    if (value(d.id, 'mode') === mode || hub.pending[point]) return;
    await act(point, mode, t('salon.surveillance.changer', { mode: t(`salon.surveillance.${mode}`) }));
  }
</script>

{#each watches as d (d.id)}
  {@const current = value(d.id, 'mode')}
  {@const asked = hub.pending[`${d.id}/mode`]}
  {@const at = since(d.id)}
  <section class="watch" class:compact class:card={!compact}>
    <div class="head">
      <h2><Icon name="shield" size={18} />{watches.length > 1 ? nameOf(d.id) : t('salon.surveillance.titre')}</h2>
      {#if d.online === false}<span class="chip">{t('salon.securite.hors_ligne')}</span>{/if}
    </div>
    <div class="modes" role="group" aria-label={t('salon.surveillance.titre')}>
      {#each MODES as [mode, icon] (mode)}
        <Tile {icon} label={t(`salon.surveillance.${mode}`)} on={current === mode} tone="cool" busy={asked === mode} dim={d.online === false} pressed={current === mode} onclick={() => choose(d, mode)} />
      {/each}
    </div>
    <p class="muted">
      {#if asked}{t('salon.surveillance.envoi')}{:else if current}{t('salon.surveillance.actuel', { mode: t(`salon.surveillance.${current}`) })}{#if at}{' · '}{t('salon.surveillance.depuis', { when: relative(at, home.now) })}{/if}{:else}{t('salon.surveillance.inconnu')}{/if}
      {' · '}{t('salon.surveillance.code')}
    </p>
  </section>
{/each}

<style>
  .watch {
    display: grid;
    gap: var(--sp-3);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .head h2 {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-base);
    font-weight: 700;
    margin: 0;
  }

  .compact .head h2 {
    font-size: var(--fs-md);
  }

  /* Three tiles side by side when the card is wide enough, else they wrap. */
  .modes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(92px, 1fr));
    gap: var(--sp-2);
  }

  p {
    margin: 0;
    font-size: var(--fs-sm);
  }
</style>
