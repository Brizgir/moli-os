<script>
  import { CARDS, cardLabel, optionsOf } from './cards.js';
  import { SIZES } from '../lib/layout.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** A card's sheet: its title, its width, and its own settings (what to
   *  show, which room or device, in what order). Nothing is kept until
   *  « Appliquer », and nothing is written until the page's « Terminé ». */
  let { card, phone = false, onclose, onapply, onduplicate, onremove } = $props();

  const def = $derived(CARDS[card.type]);
  // svelte-ignore state_referenced_locally
  let title = $state(card.title ?? '');
  // svelte-ignore state_referenced_locally
  let size = $state(card.size ?? 'm');
  // svelte-ignore state_referenced_locally
  let options = $state(JSON.parse(JSON.stringify(optionsOf(card))));

  const sizeName = { s: 'maison.modifier.taille_s', m: 'maison.modifier.taille_m', l: 'maison.modifier.taille_l' };

  /** An ordered field as rows: every choice, ticked and ranked. */
  function rows(field) {
    const choices = field.choices();
    const chosen = options[field.key];
    if (!Array.isArray(chosen)) return choices.map((c) => ({ ...c, on: true }));
    const picked = chosen.map((v) => choices.find((c) => c.value === v)).filter(Boolean);
    const rest = choices.filter((c) => !chosen.includes(c.value));
    return [...picked.map((c) => ({ ...c, on: true })), ...rest.map((c) => ({ ...c, on: false }))];
  }

  function setRows(field, list) {
    options[field.key] = list.filter((r) => r.on).map((r) => r.value);
  }

  function tick(field, value) {
    const list = rows(field).map((r) => (r.value === value ? { ...r, on: !r.on } : r));
    setRows(field, list);
  }

  function nudge(field, value, delta) {
    const list = rows(field);
    const i = list.findIndex((r) => r.value === value);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    setRows(field, list);
  }

  function apply(e) {
    e?.preventDefault();
    onapply?.({ title: title.trim() || null, size, options });
  }

  function onkey(e) {
    if (e.key === 'Escape') onclose?.();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="scrim" role="presentation" onclick={onclose}></div>
<form class="sheet" aria-label={t('maison.modifier.regler_carte', { name: cardLabel(card) })} onsubmit={apply}>
  <header>
    <h2>{cardLabel(card)}</h2>
    <button type="button" class="ghost" onclick={() => onduplicate?.()}><Icon name="copy" size={16} />{t('maison.modifier.dupliquer')}</button>
  </header>

  <label class="field">
    <span>{t('maison.modifier.titre_carte')}</span>
    <input bind:value={title} maxlength="60" placeholder={def ? t(def.label) : ''} />
  </label>

  {#if !phone}
    <div class="field">
      <span>{t('maison.modifier.taille')}</span>
      <div class="segmented" role="radiogroup" aria-label={t('maison.modifier.taille')}>
        {#each SIZES as s (s)}
          <button type="button" role="radio" aria-checked={size === s} class:on={size === s} onclick={() => (size = s)}>{t(sizeName[s])}</button>
        {/each}
      </div>
    </div>
  {/if}

  {#each def?.fields ?? [] as field (field.key)}
    {#if field.kind === 'toggle'}
      <label class="toggle">
        <span>{t(field.label)}</span>
        <input type="checkbox" checked={options[field.key] !== false} onchange={(e) => (options[field.key] = e.currentTarget.checked)} />
      </label>
    {:else if field.kind === 'pick'}
      {@const choices = field.choices()}
      <label class="field">
        <span>{t(field.label)}</span>
        <select value={options[field.key] == null ? '' : String(options[field.key])} onchange={(e) => (options[field.key] = e.currentTarget.value === '' ? null : (choices.find((c) => String(c.value) === e.currentTarget.value)?.value ?? null))}>
          {#if field.none}<option value="">{t(field.none)}</option>{:else if options[field.key] == null}<option value="">—</option>{/if}
          {#each choices as c (c.value)}
            <option value={String(c.value)}>{c.label}</option>
          {/each}
        </select>
        {#if !choices.length}<small>{t('maison.modifier.rien_a_choisir')}</small>{/if}
      </label>
    {:else if field.kind === 'order'}
      {@const list = rows(field)}
      <fieldset class="field">
        <legend>{t(field.label)}</legend>
        {#if !list.length}<small>{t('maison.modifier.rien_a_choisir')}</small>{/if}
        <ul class="order">
          {#each list as r, i (r.value)}
            <li>
              <label><input type="checkbox" checked={r.on} onchange={() => tick(field, r.value)} /><span>{r.label}</span></label>
              <button type="button" aria-label={t('maison.modifier.monter')} disabled={i === 0} onclick={() => nudge(field, r.value, -1)}><Icon name="chevron-up" size={18} /></button>
              <button type="button" aria-label={t('maison.modifier.descendre')} disabled={i === list.length - 1} onclick={() => nudge(field, r.value, 1)}><Icon name="chevron-down" size={18} /></button>
            </li>
          {/each}
        </ul>
      </fieldset>
    {/if}
  {/each}

  <div class="actions">
    <button type="button" class="ghost danger" onclick={() => onremove?.()}><Icon name="trash" size={16} />{t('maison.modifier.retirer')}</button>
    <span class="grow"></span>
    <button type="button" class="ghost" onclick={onclose}>{t('maison.modifier.annuler_nom')}</button>
    <button type="submit" class="primary">{t('maison.modifier.appliquer')}</button>
  </div>
</form>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgb(10 14 22 / 45%);
    backdrop-filter: blur(4px);
    z-index: 50;
  }

  .sheet {
    position: fixed;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    z-index: 51;
    width: min(460px, calc(100vw - 32px));
    max-height: calc(100dvh - 32px);
    overflow-y: auto;
    background: var(--surface);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-lift);
    padding: var(--sp-6);
    display: grid;
    gap: var(--sp-4);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  h2 {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: 750;
  }

  .field {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
    border: none;
    min-width: 0;
  }

  .field > span,
  legend {
    font-size: var(--fs-sm);
    font-weight: 700;
    color: var(--ink-2);
    padding: 0;
  }

  .field small {
    font-size: var(--fs-xs);
    color: var(--ink-3);
  }

  input[type='text'],
  input:not([type]),
  select {
    width: 100%;
    box-sizing: border-box;
    min-height: 48px;
    padding: 10px 14px;
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    background: var(--surface-2);
    color: var(--ink);
    font: inherit;
    font-size: var(--fs-base);
  }

  .segmented {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--sp-1);
    padding: var(--sp-1);
    border-radius: 12px;
    background: var(--surface-2);
  }

  .segmented button {
    min-height: 44px;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: var(--ink-3);
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 650;
    cursor: pointer;
  }

  .segmented button.on {
    background: var(--surface);
    color: var(--ink);
    font-weight: 700;
    box-shadow: 0 1px 2px rgb(23 32 56 / 10%);
  }

  .toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: 44px;
    font-size: 15px;
  }

  input[type='checkbox'] {
    width: 24px;
    height: 24px;
    margin: 0;
    accent-color: var(--warm);
  }

  .order {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 6px;
  }

  .order li {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 48px;
    padding: 2px 6px 2px 10px;
    border-radius: 14px;
    background: var(--surface-2);
  }

  .order label {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 44px;
    font-size: 15px;
    font-weight: 600;
    cursor: pointer;
  }

  .order li button {
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--ink-2);
    cursor: pointer;
    display: grid;
    place-items: center;
  }

  .order li button:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .grow {
    flex: 1;
  }

  .ghost,
  .primary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 44px;
    padding: 0 14px;
    border: 0;
    border-radius: 12px;
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 650;
    cursor: pointer;
  }

  .ghost {
    background: var(--surface-2);
    color: var(--ink-2);
  }

  .ghost.danger {
    color: var(--alert);
  }

  .primary {
    background: var(--warm);
    color: #33270f;
    font-weight: 700;
    padding: 0 18px;
  }
</style>
