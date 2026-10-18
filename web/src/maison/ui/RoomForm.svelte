<script>
  import { ROOM_ICONS, saveRoom, removeRoom, note } from '../lib/home.svelte.js';
  import { asHuman, CANCELLED } from '../lib/auto.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** A room's sheet: a new one (`room` null), or one to rename, give
   *  another look or other names. Saving asks the house's code once.
   *  Whether a room is protected lives in `moli.toml`: said here, not
   *  offered. */
  let { room = null, onclose } = $props();

  // The sheet edits a copy of the room, taken when it opens.
  // svelte-ignore state_referenced_locally
  let name = $state(room?.name ?? '');
  // svelte-ignore state_referenced_locally
  let icon = $state(room?.icon ?? 'home');
  // svelte-ignore state_referenced_locally
  let aliases = $state((room?.aliases ?? []).join(', '));
  let busy = $state(false);
  let error = $state('');
  let confirming = $state(false);
  const ready = $derived(name.trim().length > 0 && name.trim().length <= 60);

  async function save(e) {
    e?.preventDefault();
    if (!ready || busy) return;
    busy = true;
    error = '';
    const draft = {
      name: name.trim(),
      icon,
      aliases: aliases.split(/[,;\n]/).map((a) => a.trim()).filter(Boolean),
    };
    try {
      await asHuman(room ? t('maison.pieces.fiche_modifier', { name: room.name }) : t('maison.pieces.nouvelle'), () => saveRoom(draft, room?.name ?? null), 'piece');
      note(t('maison.pieces.enregistree', { name: draft.name }));
      onclose?.();
    } catch (err) {
      if (err.message !== CANCELLED) error = err.message;
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (busy) return;
    busy = true;
    error = '';
    try {
      await asHuman(t('maison.pieces.retirer'), () => removeRoom(room.name), 'piece');
      note(t('maison.pieces.retiree', { name: room.name }));
      onclose?.();
    } catch (err) {
      if (err.message !== CANCELLED) error = err.message;
    } finally {
      busy = false;
    }
  }

  function onkey(e) {
    if (e.key === 'Escape') onclose?.();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="scrim" role="presentation" onclick={onclose}></div>
<form class="sheet" aria-label={room ? t('maison.pieces.fiche_modifier', { name: room.name }) : t('maison.pieces.nouvelle')} onsubmit={save}>
  <h2>{room ? t('maison.pieces.fiche_modifier', { name: room.name }) : t('maison.pieces.nouvelle')}</h2>

  <label class="field">
    <span>{t('maison.pieces.nom')}</span>
    <!-- svelte-ignore a11y_autofocus -->
    <input bind:value={name} maxlength="60" placeholder={t('maison.pieces.nom_exemple')} autofocus required />
  </label>

  <fieldset class="field">
    <legend>{t('maison.pieces.icone')}</legend>
    <div class="icons" role="radiogroup" aria-label={t('maison.pieces.icone')}>
      {#each ROOM_ICONS as choice (choice)}
        <button type="button" role="radio" aria-checked={icon === choice} class:active={icon === choice} onclick={() => (icon = choice)} title={choice}>
          <Icon name={choice} size={22} />
        </button>
      {/each}
    </div>
  </fieldset>

  <label class="field">
    <span>{t('maison.pieces.alias')}</span>
    <input bind:value={aliases} placeholder={t('maison.pieces.alias_exemple')} />
    <small>{t('maison.pieces.alias_aide')}</small>
  </label>

  <p class="hint"><Icon name="lock" size={14} />{t('maison.pieces.protegee_aide')}</p>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="actions">
    <button type="button" class="ghost" onclick={onclose} disabled={busy}>{t('commun.annuler')}</button>
    <button type="submit" class="primary" disabled={!ready || busy}>{t('maison.pieces.enregistrer')}</button>
  </div>

  {#if room?.declared}
    {#if confirming}
      <p class="confirm">
        <span>{t('maison.pieces.retirer_confirmer', { name: room.name })}</span>
        <button type="button" class="danger" onclick={remove} disabled={busy}>{t('maison.pieces.retirer')}</button>
      </p>
    {:else}
      <button type="button" class="link" onclick={() => (confirming = true)} disabled={busy}>{t('maison.pieces.retirer')}</button>
    {/if}
  {/if}
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
    width: min(440px, calc(100vw - 32px));
    max-height: calc(100dvh - 32px);
    overflow-y: auto;
    background: var(--surface);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-lift);
    padding: 26px 24px 20px;
    display: grid;
    gap: 16px;
  }

  h2 {
    margin: 0;
    font-size: 20px;
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
    font-size: 13px;
    font-weight: 700;
    color: var(--ink-2);
    padding: 0;
  }

  .field small {
    font-size: 12px;
    color: var(--ink-3);
  }

  input {
    width: 100%;
    box-sizing: border-box;
    padding: 12px 14px;
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    background: var(--surface-2);
    color: var(--ink);
    font: inherit;
    font-size: 17px;
  }

  input:focus {
    outline: 2px solid var(--cool);
    outline-offset: 1px;
  }

  .icons {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(44px, 1fr));
    gap: 6px;
  }

  .icons button {
    display: grid;
    place-items: center;
    height: 44px;
    border: none;
    border-radius: 14px;
    background: var(--surface-2);
    color: var(--ink-3);
    cursor: pointer;
  }

  .icons button.active {
    background: var(--warm-soft);
    color: var(--warm-ink);
    outline: 2px solid var(--warm);
  }

  .hint {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    font-size: 13px;
    color: var(--ink-3);
  }

  .hint :global(svg) {
    flex: none;
    margin-top: 2px;
  }

  .error {
    margin: 0;
    color: var(--alert);
    font-size: 14px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .primary,
  .ghost,
  .danger {
    padding: 11px 20px;
    border-radius: 999px;
    border: none;
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }

  .primary {
    background: var(--ink);
    color: var(--surface);
  }

  .primary:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .ghost {
    background: var(--surface-2);
    color: var(--ink);
  }

  .danger {
    background: var(--alert-soft);
    color: var(--alert);
  }

  .link {
    justify-self: center;
    border: none;
    background: none;
    color: var(--ink-3);
    font: inherit;
    font-size: 13px;
    text-decoration: underline;
    cursor: pointer;
  }

  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 0;
    font-size: 14px;
    color: var(--ink-2);
  }
</style>
