<script>
  import { home, value, device } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** The way to the remote: the living room's TV (`home.json` `salon.tv`),
   *  or the one chosen (`device`). */
  let { device: chosen = null, title = null } = $props();

  const tv = $derived(chosen ?? home.config?.salon?.tv ?? null);
  const speaker = $derived(home.config?.salon?.speaker);
  const tvOn = $derived(value(tv, 'power') === true);
  const playing = $derived(value(speaker, 'playing') === true);
</script>

{#if tv && device(tv)}
  <a class="card remote" href="#/salon">
    <span class="tv-object" aria-hidden="true">
      <span class="screen" class:lit={tvOn}></span>
      <span class="stand"></span>
    </span>
    <span class="remote-text">
      <strong>{title ?? t('maison.accueil.telecommande')}</strong>
      <span class="muted">{tvOn ? t('maison.accueil.tele_allumee') : t('maison.accueil.tele_veille')}{playing ? ` · ${t('maison.accueil.son_joue')}` : ''}</span>
    </span>
    <span class="go"><Icon name="arrow-top-right" size={20} /></span>
  </a>
{/if}

<style>
  .remote {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    color: inherit;
    text-decoration: none;
    min-height: 168px;
    align-content: space-between;
    transition: transform 0.2s var(--ease);
  }

  .remote:hover {
    transform: translateY(-2px);
  }

  .tv-object {
    grid-column: 1 / -1;
    display: grid;
    justify-items: center;
  }

  .screen {
    width: 96px;
    height: 56px;
    border-radius: 8px;
    background: #171c28;
    box-shadow: inset 0 0 0 3px #2a3142;
    transition: background 0.4s;
  }

  .screen.lit {
    background: linear-gradient(135deg, #5fb4f0, #8f8cf5 55%, #f0a53a);
    box-shadow: inset 0 0 0 3px #2a3142, 0 8px 26px -6px #8f8cf5aa;
  }

  .stand {
    width: 30px;
    height: 6px;
    border-radius: 0 0 6px 6px;
    background: #2a3142;
  }

  .remote-text {
    display: grid;
  }

  .remote-text strong {
    font-size: 18px;
  }

  .remote-text span {
    font-size: 13px;
  }

  .go {
    align-self: end;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--surface-2);
    display: grid;
    place-items: center;
    color: var(--ink-2);
  }
</style>
