<script>
  import { hub, value } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import Gauge from '../ui/infra/Gauge.svelte';

  /** The house's server at a glance: processor, memory, temperature, up
   *  since when; a tap opens the machines' page. No server device: nothing. */
  let { title = null } = $props();

  const server = $derived(Object.values(hub.devices).find((d) => d.model === 'Serveur de la maison'));
  const sv = (k) => (server ? value(server.id, k) : null);
  function since(seconds) {
    if (typeof seconds !== 'number') return '—';
    const d = Math.floor(seconds / 86400);
    const h = Math.floor((seconds % 86400) / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    return d ? t('systeme.format.duree_jours', { d, h }) : h ? t('systeme.format.duree_heures', { h, m }) : t('systeme.format.duree_minutes', { m });
  }
</script>

{#if server}
  <a class="card machine" href="#/infra">
    <div class="card-head">
      <h2><Icon name="server" size={18} />{title ?? t('maison.cartes.machine')}</h2>
      <span class="more" aria-hidden="true"><Icon name="arrow-top-right" size={18} /></span>
    </div>
    <div class="gauges">
      <Gauge label={t('systeme.infra.processeur')} value={sv('cpu')} warn={[75, 92]} />
      <Gauge label={t('systeme.infra.memoire')} value={sv('ram')} warn={[85, 95]} />
      <Gauge label={t('systeme.infra.temperature')} value={sv('cpu_temperature')} unit="°" max={100} warn={[75, 85]} />
    </div>
    <p class="muted small">{server.online === false ? t('commun.piece.hors_ligne', { name: '' }) : t('systeme.infra.serveur', { since: since(sv('uptime')) })}</p>
  </a>
{/if}

<style>
  .machine {
    display: grid;
    color: inherit;
    text-decoration: none;
  }

  .gauges {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--sp-2);
  }

  .small {
    font-size: var(--fs-sm);
    margin: 10px 0 0;
  }
</style>
