<script>
  import { act, value, nameOf, home } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** Pairing a TV that shows a code (Android TV / Google TV: 6 characters,
   *  0-9 and A-F). The TV's `pairing` point says where it is; writing it
   *  `code` asks the TV for a code; the code typed goes to `pairing_code`, a
   *  « secret » point: a person's, at the dashboard, never journaled nor
   *  kept. Nothing shows once the TV is paired. `named`: the TV's name in
   *  the title (several TVs on one page). */
  let { tv, named = false } = $props();

  const state = $derived(value(tv, 'pairing'));
  let code = $state('');
  let busy = $state(false);
  let error = $state('');
  const ready = $derived(/^[0-9a-f]{6}$/i.test(code.trim()));

  function ask() {
    error = '';
    act(`${tv}/pairing`, 'code', t('salon.appairage.afficher'));
  }

  async function send() {
    busy = true;
    error = '';
    try {
      const res = await fetch('/api/command', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ point: `${tv}/pairing_code`, value: code.trim() }),
      });
      if (res.status === 403) {
        // A person's code first; the same step goes again after it.
        home.held = { label: t('salon.appairage.appairer'), reason: t('salon.appairage.ton_code'), custom: send };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        error = body.error ?? res.statusText;
        return;
      }
      code = '';
    } catch (e) {
      error = e.message;
    } finally {
      busy = false;
    }
  }
</script>

{#if state != null && state !== 'paired'}
  <section class="pairing" aria-label={t('salon.appairage.titre')}>
    <div class="head">
      <span class="ico"><Icon name="tv" size={18} /></span>
      <b>{named ? t('salon.appairage.titre_nom', { name: nameOf(tv) }) : t('salon.appairage.titre')}</b>
    </div>
    {#if state === 'code'}
      <p class="muted">{t('salon.appairage.taper')}</p>
      <form onsubmit={(e) => (e.preventDefault(), ready && send())} autocomplete="off">
        <input
          bind:value={code}
          maxlength="6"
          inputmode="text"
          autocapitalize="characters"
          spellcheck="false"
          aria-label={t('salon.appairage.code')}
          placeholder="A1B2C3" />
        <button class="btn primary" disabled={busy || !ready}>{busy ? '…' : t('salon.appairage.appairer')}</button>
      </form>
      <button class="link" onclick={ask}>{t('salon.appairage.nouveau')}</button>
    {:else if state === 'waiting'}
      <p class="muted">{t('salon.appairage.attente')}</p>
    {:else}
      <p class="muted">{t('salon.appairage.aide')}</p>
      <button class="btn primary" onclick={ask}>{t('salon.appairage.afficher')}</button>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </section>
{/if}

<style>
  .pairing {
    display: grid;
    gap: 10px;
    padding: 14px 16px;
    border-radius: var(--r-md);
    background: var(--warm-soft);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .ico {
    display: grid;
    place-items: center;
  }

  p {
    margin: 0;
  }

  form {
    display: flex;
    gap: 8px;
  }

  input {
    flex: 1;
    min-width: 0;
    font: inherit;
    font-size: 20px;
    letter-spacing: 0.25em;
    text-transform: uppercase;
    padding: 8px 12px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line, var(--surface-3));
    background: var(--surface);
    color: inherit;
  }

  .link {
    justify-self: start;
    border: 0;
    background: none;
    padding: 0;
    color: var(--cool);
    font: inherit;
    cursor: pointer;
  }

  .error {
    color: var(--danger, #c0392b);
  }
</style>
