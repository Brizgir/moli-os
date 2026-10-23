<script>
  import { moli, suggestions } from '../lib/moli.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Orb from '../moli/Orb.svelte';
  import MoliSheet from '../moli/MoliSheet.svelte';

  /** Moli on the home page: the orb, a word to start, and the ideas of the
   *  moment; a tap opens the conversation. */
  let { title = null } = $props();

  let open = $state(false);
  const ideas = $derived(suggestions(3));
</script>

<section class="card assistant">
  <button class="ask" onclick={() => (open = true)}>
    <Orb size={44} state={moli.busy ? 'thinking' : 'idle'} />
    <span class="words">
      <b>{title ?? t('commun.demander_moli')}</b>
      <small>{t('maison.cartes.assistant_aide')}</small>
    </span>
  </button>
  {#if ideas.length}
    <div class="ideas">
      {#each ideas as idea (idea)}
        <button class="idea" onclick={() => (open = true)}>{idea}</button>
      {/each}
    </div>
  {/if}
</section>

{#if open}
  <MoliSheet onclose={() => (open = false)} />
{/if}

<style>
  .assistant {
    display: grid;
    gap: var(--sp-3);
  }

  .ask {
    display: flex;
    align-items: center;
    gap: 14px;
    border: 0;
    background: transparent;
    padding: 0;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    min-height: 44px;
  }

  .words {
    display: grid;
  }

  .words b {
    font-size: 17px;
  }

  .words small {
    font-size: var(--fs-sm);
    color: var(--ink-3);
  }

  .ideas {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }

  .idea {
    border: 0;
    border-radius: 999px;
    padding: 8px 12px;
    background: var(--surface-2);
    color: var(--ink-2);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 600;
    cursor: pointer;
    min-height: 36px;
  }
</style>
