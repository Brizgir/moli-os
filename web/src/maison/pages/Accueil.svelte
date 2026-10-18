<script>
  import { onMount } from 'svelte';
  import { hub, home, note } from '../lib/home.svelte.js';
  import { asHuman, CANCELLED } from '../lib/auto.svelte.js';
  import { dashboard, startDashboard, loadDashboard, current, canEdit, beginEditing, draftCards } from '../lib/dashboard.svelte.js';
  import { move, shift, update, setHidden, add, remove, duplicate, defaultLayout, SPAN } from '../lib/layout.js';
  import { CARDS, cardLabel } from '../accueil/cards.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import Card from '../accueil/Card.svelte';
  import Chrome from '../accueil/Chrome.svelte';
  import Toolbar from '../accueil/Toolbar.svelte';
  import CardSettings from '../accueil/CardSettings.svelte';
  import Catalogue from '../accueil/Catalogue.svelte';

  // The home page is a list of cards, arranged by the person (the house's
  // arrangement, or their own; `dashboards.json`), laid out as a grid that
  // packs densely: a card has a width (a third, two thirds, the row), never
  // a position. Without any arrangement, the list is the page as it always
  // was. Each card still shows only when what it tells exists here.
  onMount(startDashboard);

  const editing = $derived(!!dashboard.editing);
  const phone = $derived(dashboard.screen === 'phone');
  const resolved = $derived(current());
  // While a card is dragged, the list follows the finger before it is kept.
  let drag = $state(null);
  const cards = $derived(drag?.cards ?? resolved.cards);
  const shown = $derived(cards.filter((c) => !c.hidden));
  const hiddenOnes = $derived(cards.filter((c) => c.hidden));

  // The button shows to the household; a guest only looks.
  const guest = $derived(hub.session?.person?.role === 'guest');

  /** « Modifier »: with the right already, at once; else the code is asked
   *  (a person of the house), then the right is read again. */
  async function edit() {
    if (canEdit()) {
      beginEditing();
      return;
    }
    try {
      await asHuman(t('maison.modifier.titre'), async () => {
        await loadDashboard();
        if (!canEdit()) throw new Error(t('maison.modifier.refuse'));
      }, 'disposition');
      beginEditing();
    } catch (err) {
      if (err.message !== CANCELLED) note(err.message, 'error');
    }
  }

  // ---- changes to the draft ---------------------------------------------------------

  let settings = $state(null); // the card being set
  let catalogue = $state(false);

  const change = (next) => draftCards(next);

  function applySettings(id, patch) {
    change(update(cards, id, patch));
    settings = null;
  }

  function addEntry(entry) {
    const def = CARDS[entry.type];
    change(add(cards, { type: entry.type, size: phone ? 'l' : def && ['room', 'speaker', 'diffuser', 'camera', 'printer', 'presence', 'assistant', 'machine'].includes(entry.type) ? 's' : 'm', options: { ...(def?.options?.() ?? {}), ...entry.options } }));
    catalogue = false;
    note(t('maison.modifier.ajoutee', { name: entry.label ?? t(def?.label ?? entry.type) }));
  }

  function reset() {
    change(defaultLayout(dashboard.screen).cards);
  }

  // ---- dragging a card by its handle (mouse or finger) ------------------------------

  function dragStart(id, e) {
    if (e.button != null && e.button !== 0) return;
    e.preventDefault();
    drag = { id, cards: cards.slice(), y: e.clientY };
    e.currentTarget.setPointerCapture?.(e.pointerId);
    window.addEventListener('pointermove', dragMove);
    window.addEventListener('pointerup', dragEnd);
    window.addEventListener('pointercancel', dragEnd);
  }

  function dragMove(e) {
    if (!drag) return;
    drag.y = e.clientY;
    // Near an edge of the window, the page scrolls along.
    const edge = 72;
    if (e.clientY < edge) window.scrollBy(0, -14);
    else if (e.clientY > window.innerHeight - edge) window.scrollBy(0, 14);
    const over = document.elementsFromPoint(e.clientX, e.clientY).find((el) => el.dataset?.card && el.dataset.card !== drag.id);
    if (!over) return;
    const from = drag.cards.findIndex((c) => c.id === drag.id);
    const to = drag.cards.findIndex((c) => c.id === over.dataset.card);
    if (from >= 0 && to >= 0) drag.cards = move(drag.cards, from, to);
  }

  function dragEnd() {
    window.removeEventListener('pointermove', dragMove);
    window.removeEventListener('pointerup', dragEnd);
    window.removeEventListener('pointercancel', dragEnd);
    if (!drag) return;
    const moved = drag.cards;
    const was = resolved.cards;
    drag = null;
    if (moved.some((c, i) => c.id !== was[i]?.id)) change(moved);
  }

  const span = (card) => (phone ? 1 : (SPAN[card.size] ?? 2));
</script>

<div class="accueil" class:editing>
  {#if editing}
    <Toolbar ondefault={reset} oncatalogue={() => (catalogue = true)} />
  {:else if !guest}
    <div class="edit-row">
      <button class="edit" onclick={edit} aria-label={t('maison.modifier.bouton')} title={t('maison.modifier.bouton')}>
        <Icon name="pencil" size={18} /><span>{t('maison.modifier.bouton')}</span>
      </button>
    </div>
  {/if}

  <div class="grid" class:editing>
    {#each shown as card, i (card.id)}
      {#if editing}
        <div class="slot edit s{span(card)}" class:dragging={drag?.id === card.id} data-card={card.id}>
          <Chrome
            {card}
            first={i === 0}
            last={i === shown.length - 1}
            {phone}
            ondrag={(e) => dragStart(card.id, e)}
            onsize={(size) => change(update(cards, card.id, { size }))}
            onhide={() => change(setHidden(cards, card.id, true))}
            onsettings={() => (settings = card.id)}
            onshift={(delta) => change(shift(cards, card.id, delta))}
          />
          {#if CARDS[card.type]}
            <div class="preview" aria-hidden="true" inert>
              <Card {card} screen={dashboard.screen} />
            </div>
          {/if}
        </div>
      {:else}
        <div class="slot s{span(card)}">
          <Card {card} screen={dashboard.screen} />
        </div>
      {/if}
    {/each}
  </div>

  {#if editing}
    <section class="hidden-tray" aria-label={t('maison.modifier.masquees')}>
      <h2>{hiddenOnes.length ? t('maison.modifier.masquees_aide') : t('maison.modifier.masquees_aucune')}</h2>
      {#if hiddenOnes.length}
        <div class="chips">
          {#each hiddenOnes as card (card.id)}
            <button onclick={() => change(setHidden(cards, card.id, false))}><Icon name="plus" size={16} />{cardLabel(card)}</button>
          {/each}
        </div>
      {/if}
    </section>
  {/if}
</div>

{#if settings}
  {@const card = cards.find((c) => c.id === settings)}
  {#if card}
    <CardSettings
      {card}
      {phone}
      onclose={() => (settings = null)}
      onapply={(patch) => applySettings(card.id, patch)}
      onduplicate={() => {
        change(duplicate(cards, card.id));
        settings = null;
      }}
      onremove={() => {
        change(remove(cards, card.id));
        settings = null;
      }}
    />
  {/if}
{/if}

{#if catalogue}
  <Catalogue {cards} onadd={addEntry} onclose={() => (catalogue = false)} />
{/if}

<style>
  .accueil {
    display: grid;
    gap: 22px;
  }

  .edit-row {
    display: flex;
    justify-content: flex-end;
    margin-bottom: -14px;
  }

  .edit {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 16px;
    border: 0;
    border-radius: 18px;
    background: var(--surface);
    color: var(--ink-2);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--shadow);
  }

  /* Three columns that pack densely: a card spans a third, two thirds or
     the row, and the next small one fills what is left. */
  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    grid-auto-flow: dense;
    gap: 20px;
    align-items: start;
  }

  /* A slot is nothing by itself: its card (when it shows) is the grid's
     item, so a silent card leaves no hole. */
  .slot {
    display: contents;
  }

  .slot > :global(*) {
    min-width: 0;
    margin-top: 0;
  }

  .slot.s1 > :global(*) {
    grid-column: span 1;
  }

  .slot.s2 > :global(*) {
    grid-column: span 2;
  }

  .slot.s3 > :global(*) {
    grid-column: span 3;
  }

  /* While changing: the slot is the item, with the card's handle and a
     glimpse of it. */
  .slot.edit {
    display: grid;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--r-lg);
    background: var(--surface);
    outline: 2px dashed var(--line);
    outline-offset: -2px;
    min-width: 0;
  }

  .slot.edit.s1 {
    grid-column: span 1;
  }

  .slot.edit.s2 {
    grid-column: span 2;
  }

  .slot.edit.s3 {
    grid-column: span 3;
  }

  .slot.edit > :global(*) {
    grid-column: auto;
  }

  .slot.dragging {
    outline: 3px solid var(--warm);
    box-shadow: var(--shadow-lift);
    z-index: 2;
  }

  .preview {
    max-height: 150px;
    overflow: hidden;
    opacity: 0.55;
    pointer-events: none;
    mask-image: linear-gradient(#000 55%, transparent);
    border-radius: var(--r-md);
  }

  .preview > :global(*) {
    margin-top: 0;
  }

  .hidden-tray {
    display: grid;
    gap: 10px;
  }

  .hidden-tray h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 700;
    color: var(--ink-3);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .chips button {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 16px;
    border: 1px dashed var(--ink-3);
    border-radius: 14px;
    background: transparent;
    color: var(--ink-2);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }

  @media (max-width: 1179.98px) {
    .grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .slot.s3 > :global(*),
    .slot.edit.s3,
    .slot.edit.s2 {
      grid-column: span 2;
    }

    .slot.edit.s3 > :global(*),
    .slot.edit.s2 > :global(*) {
      grid-column: auto;
    }
  }

  @media (max-width: 767.98px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
      gap: 16px;
    }

    .slot > :global(*),
    .slot.edit {
      grid-column: span 1 !important;
    }

    .slot.edit > :global(*) {
      grid-column: auto !important;
    }

    .preview {
      display: none;
    }

    .edit span {
      display: none;
    }

    .edit {
      padding: 0 13px;
    }
  }
</style>
