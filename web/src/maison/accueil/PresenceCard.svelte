<script>
  import { onMount } from 'svelte';
  import { home, relative } from '../lib/home.svelte.js';
  import { people, loadPeople, whereabouts, colorOf } from '../lib/people.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import Tile from '../ui/Tile.svelte';

  /** Who is home, from the phones: a tile per person, where they are and
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
    <div class="people">
      {#each list as p (p.id)}
        {@const w = whereabouts(p.id)}
        <Tile row static glyph={p.name.slice(0, 1).toUpperCase()} label={p.name} state={`${place(p)}${w.since ? ` · ${relative(w.since, home.now)}` : ''}`} on={w.home === true} tone="good" tint={colorOf(p)} dim={w.home === false} />
      {/each}
    </div>
  </section>
{/if}

<style>
  .people {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--sp-2);
  }

  .muted {
    font-size: var(--fs-sm);
  }
</style>
