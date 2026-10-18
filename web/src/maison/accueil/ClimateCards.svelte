<script>
  import { hub } from '../lib/home.svelte.js';
  import { climates } from './cards.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import ClimateCard from '../ui/ClimateCard.svelte';

  /** The clims (and radiators that are not a heated room), each with its
   *  dial. `devices`: the ones to show, in that order (null: all). The
   *  section is named after what it lists: radiators and thermostats (a
   *  `heating` point), air conditioners, or both. */
  let { devices = null, title = null } = $props();

  const all = $derived(climates());
  const clims = $derived(devices ? devices.map((id) => all.find((c) => c.id === id)).filter(Boolean) : all);
  const heaters = $derived(clims.map((c) => hub.devices[c.id]).map((d) => d.points.some((p) => p.key === 'heating')));
  const climateTitle = $derived(
    heaters.length && heaters.every(Boolean) ? 'maison.accueil.chauffage' : heaters.some(Boolean) ? 'maison.accueil.temperature' : 'maison.accueil.clim',
  );
  const climateIcon = $derived(heaters.length && heaters.every(Boolean) ? 'radiator' : 'air-conditioner');
</script>

{#if clims.length}
  <section class="card climate">
    <div class="card-head"><h2><Icon name={climateIcon} size={18} />{title ?? t(climateTitle)}</h2></div>
    <div class="clims">
      {#each clims as c (c.id)}
        <ClimateCard id={c.id} name={c.name} />
      {/each}
    </div>
  </section>
{/if}

<style>
  .clims {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  @media (max-width: 767.98px) {
    .clims {
      grid-template-columns: 1fr;
    }
  }
</style>
