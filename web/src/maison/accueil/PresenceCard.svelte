<script>
  import { onMount } from 'svelte';
  import { home, relative } from '../lib/home.svelte.js';
  import { people, loadPeople, whereabouts, colorOf } from '../lib/people.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** Who is home, from the phones: a dot per person, where they are and
   *  since when. Nobody declared: nothing. */
  let { title = null } = $props();

  onMount(() => {
    if (!people.loaded) loadPeople();
  });

  const list = $derived(people.list.filter((p) => p.role !== 'guest'));
  const atHome = $derived(list.filter((p) => whereabouts(p.id).home === true).length);

  function place(p) {
    const w = whereabouts(p.id);
    if (w.home === true) return t('personnes.carte.a_la_maison');
    if (w.home === false) return w.zone ?? t('maison.cartes.presence_absent');
    return t('personnes.carte.inconnu');
  }
</script>

{#if list.length}
  <section class="card presence">
    <div class="card-head">
      <h2><Icon name="people" size={18} />{title ?? t('maison.cartes.presence')}</h2>
      <span class="muted">{t('maison.cartes.presence_compte', { count: atHome })}</span>
    </div>
    <ul>
      {#each list as p (p.id)}
        {@const w = whereabouts(p.id)}
        <li class:home={w.home === true}>
          <span class="dot" style:background={colorOf(p)}>{p.name.slice(0, 1).toUpperCase()}</span>
          <span class="who"><b>{p.name}</b><small>{place(p)}{#if w.since} · {relative(w.since, home.now)}{/if}</small></span>
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
    gap: 10px;
  }

  li {
    display: flex;
    align-items: center;
    gap: 12px;
    opacity: 0.6;
  }

  li.home {
    opacity: 1;
  }

  .dot {
    flex: none;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: #fff;
    font-weight: 700;
  }

  .who {
    display: grid;
    min-width: 0;
  }

  .who small {
    font-size: 12.5px;
    color: var(--ink-3);
  }

  .muted {
    font-size: 13px;
  }
</style>
