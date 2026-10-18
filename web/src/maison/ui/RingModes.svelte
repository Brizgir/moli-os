<script>
  import { hub, home, value, act, hidden, relative, nameOf } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

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
    <div class="modes" role="radiogroup" aria-label={t('salon.surveillance.titre')}>
      {#each MODES as [mode, icon] (mode)}
        <button role="radio" aria-checked={current === mode} class:on={current === mode} class:asked={asked === mode} disabled={d.online === false} onclick={() => choose(d, mode)}>
          <Icon name={icon} size={18} />
          <span>{t(`salon.surveillance.${mode}`)}</span>
        </button>
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
    gap: 12px;
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
    gap: 8px;
    font-size: 16px;
    font-weight: 700;
    margin: 0;
  }

  .compact .head h2 {
    font-size: 14px;
  }

  .modes {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .modes button {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 8px;
    border-radius: 14px;
    border: 2px solid transparent;
    background: var(--surface-2);
    color: var(--ink-2);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    min-height: 44px;
  }

  .compact .modes button {
    flex-direction: row;
    padding: 10px 8px;
    font-size: 13px;
  }

  .modes button:focus-visible {
    outline: 3px solid var(--cool);
    outline-offset: 2px;
  }

  .modes button.on {
    background: var(--cool-soft);
    color: var(--cool);
    border-color: var(--cool);
  }

  .modes button.asked {
    border-style: dashed;
    border-color: var(--ink-3);
  }

  .modes button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  p {
    margin: 0;
    font-size: 13px;
  }
</style>
