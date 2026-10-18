<script>
  import { device, nameOf } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import CameraTile from '../ui/CameraTile.svelte';

  /** One camera's image on the home page, refreshed every `refresh`
   *  seconds (a battery camera: at a touch only). */
  let { device: id = null, refresh = 15, title = null } = $props();

  const found = $derived(id ? device(id) : null);
  const battery = $derived(!!found?.points.some((p) => p.key === 'battery'));
</script>

{#if found}
  <section class="card camera">
    <div class="card-head">
      <h2><Icon name="cctv" size={18} />{title ?? nameOf(id)}</h2>
      <a class="more" href="#/cameras" aria-label={t('maison.accueil.toutes_cameras')}><Icon name="arrow-top-right" size={18} /></a>
    </div>
    <CameraTile {id} name={nameOf(id)} refresh={battery ? 0 : refresh} {battery} />
  </section>
{/if}
