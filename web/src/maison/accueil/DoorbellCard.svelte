<script>
  import { onMount } from 'svelte';
  import { home, hub, value, nameOf, hidden, relative, lastSeen, ringing } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import CameraTile from '../ui/CameraTile.svelte';
  import DoorbellEvents from '../ui/DoorbellEvents.svelte';
  import RingModes from '../ui/RingModes.svelte';

  /** The door: the layout's first camera (or the one chosen, `camera`),
   *  else a doorbell with an image (Ring, on battery: its last image, a
   *  touch asks a fresh one), else a doorbell without image (its events),
   *  and the watch's modes (`modes`). */
  let { camera = null, modes = true, title = null } = $props();

  const cfg = $derived(home.config ?? {});
  const bell = $derived.by(() => {
    if (camera) {
      const d = hub.devices[camera];
      return d ? { id: d.id, name: nameOf(d.id), battery: d.points.some((p) => p.key === 'battery') } : undefined;
    }
    const listed = (cfg.favorites?.cameras ?? [])[0];
    if (listed) return listed;
    const d = Object.values(hub.devices).find((d) => d.camera && !hidden(d.id) && d.points.some((p) => p.key === 'doorbell'));
    return d ? { id: d.id, name: nameOf(d.id), battery: d.points.some((p) => p.key === 'battery') } : undefined;
  });
  const rings = $derived(Object.values(hub.devices).some((d) => !hidden(d.id) && ringing(d)));

  /** A tap on the image: the camera big, with its passages of the day. */
  const openBig = (id) => (location.hash = `#/cameras/${encodeURIComponent(id)}`);

  // The last real passage at the door (the history knows; the live value
  // is refreshed every few seconds whatever happens).
  let passage = $state(null);
  onMount(() => {
    const refresh = () => bell && lastSeen(bell.id).then((ts) => (passage = ts));
    const timer = setInterval(refresh, 120_000);
    return () => clearInterval(timer);
  });
  $effect(() => {
    if (bell) lastSeen(bell.id).then((ts) => (passage = ts));
  });
</script>

{#if bell}
  <section class="card doorbell">
    <div class="card-head">
      <h2><Icon name="bell" size={18} />{title ?? t('maison.accueil.devant_porte')}</h2>
      <a class="more" href="#/cameras" aria-label={t('maison.accueil.toutes_cameras')}><Icon name="arrow-top-right" size={18} /></a>
    </div>
    <CameraTile id={bell.id} name={bell.name} refresh={bell.battery ? 0 : 15} battery={!!bell.battery} onopen={openBig} />
    <p class="muted small">{passage ? t('maison.accueil.dernier_passage', { when: relative(passage, home.now) }) : t('maison.accueil.personne')}</p>
    {#if modes}<RingModes compact />{/if}
  </section>
{:else if rings}
  <div class="doorbell events"><DoorbellEvents />{#if modes}<RingModes compact />{/if}</div>
{:else if modes}
  <RingModes />
{/if}

<style>
  .small {
    font-size: var(--fs-sm);
    margin-top: 10px;
  }

  .events {
    display: grid;
    gap: var(--sp-5);
  }
</style>
