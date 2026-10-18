<script>
  import { CARDS, cardLabel, cardSummary } from './cards.js';
  import { SIZES } from '../lib/layout.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** A card's handle while the page is being changed: its name, its width
   *  (S, M, L), the arrows, hide, settings. The handle starts a drag
   *  (`ondrag(event)`); everything is 44 px, for a finger. */
  let { card, first = false, last = false, phone = false, ondrag, onsize, onhide, onsettings, onshift } = $props();

  const def = $derived(CARDS[card.type]);
  const sizeName = { s: 'maison.modifier.taille_s', m: 'maison.modifier.taille_m', l: 'maison.modifier.taille_l' };
</script>

<div class="chrome">
  <button class="handle" aria-label={t('maison.modifier.deplacer', { name: cardLabel(card) })} onpointerdown={(e) => ondrag?.(e)}>
    <Icon name="drag" size={22} />
  </button>
  <div class="who">
    <b><Icon name={def?.icon ?? 'alert'} size={15} />{cardLabel(card)}</b>
    <small>{cardSummary(card)}</small>
  </div>
  {#if !phone}
    <div class="sizes" role="radiogroup" aria-label={t('maison.modifier.taille')}>
      {#each SIZES as s (s)}
        <button role="radio" aria-checked={card.size === s} class:on={card.size === s} onclick={() => onsize?.(s)} title={t(sizeName[s])}>{s.toUpperCase()}</button>
      {/each}
    </div>
  {/if}
  <div class="arrows">
    <button aria-label={t('maison.modifier.monter')} disabled={first} onclick={() => onshift?.(-1)}><Icon name="chevron-up" size={18} /></button>
    <button aria-label={t('maison.modifier.descendre')} disabled={last} onclick={() => onshift?.(1)}><Icon name="chevron-down" size={18} /></button>
  </div>
  <button class="round" aria-label={t('maison.modifier.masquer')} onclick={() => onhide?.()}><Icon name="eye-off" size={20} /></button>
  <button class="round" aria-label={t('maison.modifier.regler')} onclick={() => onsettings?.()} disabled={!def}><Icon name="cog" size={20} /></button>
</div>

<style>
  .chrome {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  button {
    border: 0;
    background: var(--surface-2);
    color: var(--ink-2);
    font: inherit;
    cursor: pointer;
    display: grid;
    place-items: center;
  }

  button:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .handle {
    flex: none;
    width: 44px;
    height: 44px;
    border-radius: 12px;
    color: var(--ink-3);
    cursor: grab;
    touch-action: none;
  }

  .who {
    flex: 1;
    min-width: 0;
    display: grid;
  }

  .who b {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .who small {
    font-size: 12.5px;
    color: var(--ink-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sizes {
    flex: none;
    display: flex;
    gap: 2px;
    padding: 4px;
    border-radius: 12px;
    background: var(--surface-2);
  }

  .sizes button {
    min-width: 40px;
    height: 36px;
    border-radius: 9px;
    background: transparent;
    font-size: 13px;
    font-weight: 650;
    color: var(--ink-3);
  }

  .sizes button.on {
    background: var(--surface);
    color: var(--ink);
    font-weight: 700;
    box-shadow: 0 1px 2px rgb(23 32 56 / 10%);
  }

  .arrows {
    flex: none;
    display: grid;
    gap: 2px;
  }

  .arrows button {
    width: 44px;
    height: 21px;
    border-radius: 7px;
  }

  .round {
    flex: none;
    width: 44px;
    height: 44px;
    border-radius: 12px;
  }

  @media (max-width: 767.98px) {
    .chrome {
      gap: 6px;
    }
  }
</style>
