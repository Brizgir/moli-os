<script>
  import { onMount } from 'svelte';
  import { home, clock } from '../lib/home.svelte.js';
  import { timed, useTimed } from '../lib/timed.svelte.js';
  import { timedOn } from '../lib/today.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** `title`: the person's own, from the home page's arrangement. */
  let { title = null } = $props();

  /** « Aujourd'hui »: the automations with a time of day, today, by time;
   *  those already past are ticked. Nothing timed today: no card. */
  onMount(useTimed);

  const list = $derived(timedOn(timed.list, new Date(home.now)));
  const at = (minutes) => {
    const d = new Date(home.now);
    d.setHours(Math.floor(minutes / 60), minutes % 60, 0, 0);
    return clock(d);
  };
</script>

{#if list.length}
  <section class="card today">
    <div class="card-head">
      <h2><Icon name="clock" size={18} />{title ?? t('maison.accueil.aujourdhui')}</h2>
      <a class="more" href="#/automatismes" aria-label={t('automatismes.routines.tous')}><Icon name="arrow-top-right" size={18} /></a>
    </div>
    <ul>
      {#each list as x (`${x.id}/${x.minutes}`)}
        <li class:done={x.done}>
          <span>{x.name}</span>
          <span class="when num">
            {at(x.minutes)}{#if x.done}<span class="tick" role="img" aria-label={t('maison.accueil.fait')}><Icon name="ok" size={14} /></span>{/if}
          </span>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 12px;
  }

  li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 14.5px;
  }

  li span:first-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .when {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-weight: 650;
  }

  .done .when {
    font-weight: 500;
    color: var(--ink-3);
  }

  .tick {
    display: inline-grid;
    color: var(--good);
  }
</style>
