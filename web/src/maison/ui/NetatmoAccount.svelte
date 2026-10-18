<script>
  import { hub, home, value, note } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** Each Netatmo Energy account Moli drives: the app's keys first (made by
   *  the person on dev.netatmo.com, filed encrypted, never shown again),
   *  then Netatmo's authorization page and its return address pasted here.
   *  Netatmo's page takes the password; Moli only gets the code. */
  const accounts = $derived(
    Object.values(hub.devices ?? {}).filter((d) => d.manufacturer === 'Netatmo' && d.model === 'Energy API' && d.points?.some((p) => p.key === 'sign_in')),
  );
  const roomsOf = (account) =>
    Object.values(hub.devices ?? {}).filter((d) => d.instance === account.instance && d.points?.some((p) => p.key === 'setpoint')).length;

  let pasted = $state('');
  let clientId = $state('');
  let clientSecret = $state('');
  let showKeys = $state(false);
  let busy = $state(false);
  let error = $state('');

  async function connect(id) {
    busy = true;
    error = '';
    try {
      const res = await fetch('/api/command', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ point: `${id}/sign_in`, value: pasted.trim() }),
      });
      if (res.status === 403) {
        home.held = { label: t('systeme.netatmo.connecter'), reason: 'ton code', custom: () => connect(id) };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        error = body.error ?? res.statusText;
        return;
      }
      pasted = '';
    } catch (e) {
      error = e.message;
    } finally {
      busy = false;
    }
  }

  async function saveKeys(instance) {
    busy = true;
    error = '';
    try {
      const res = await fetch('/api/integrations/netatmo-app', {
        method: 'PUT',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ instance, client_id: clientId.trim(), client_secret: clientSecret.trim() }),
      });
      if (res.status === 403) {
        // A person's gesture: the code first, then the same request again.
        home.held = { label: t('systeme.netatmo.cles_titre'), reason: 'ton code', custom: () => saveKeys(instance) };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.error ?? res.statusText);
      clientId = '';
      clientSecret = '';
      showKeys = false;
      note(t('systeme.netatmo.cles_enregistrees'), 'info');
    } catch (e) {
      error = e.message;
    } finally {
      busy = false;
    }
  }
</script>

{#each accounts as a (a.id)}
  {@const status = value(a.id, 'status')}
  {@const url = value(a.id, 'sign_in_url')}
  {@const redirect = value(a.id, 'redirect_uri') || 'http://localhost/moli-netatmo'}
  <section class="card" class:done={status === 'connected'}>
    <div class="row">
      <span class="ico"><Icon name="thermometer" size={18} /></span>
      <div class="text">
        <b>{t('systeme.netatmo.titre')}</b>
        <span class="muted">
          {#if status === 'connected'}
            {t('systeme.netatmo.connecte', { n: roomsOf(a) })}
          {:else if status === 'app_missing'}
            {t('systeme.netatmo.app_manquante')}
          {:else if status === 'sign_in'}
            {t('systeme.netatmo.a_connecter')}
          {:else if status === 'app_refused'}
            {t('systeme.netatmo.app_refusee')}
          {:else if status === 'quota'}
            {t('systeme.netatmo.quota')}
          {:else if status === 'error'}
            {t('systeme.netatmo.erreur')}
          {:else}
            {t('systeme.netatmo.connexion')}
          {/if}
        </span>
      </div>
      {#if status === 'connected'}
        <a class="btn" href="#/chauffage">{t('maison.chauffage.lien')}</a>
      {/if}
      {#if status !== 'app_missing'}
        <button class="btn" onclick={() => (showKeys = !showKeys)}>{t('systeme.netatmo.changer_app')}</button>
      {/if}
    </div>

    {#if status === 'app_missing' || status === 'app_refused' || showKeys}
      {#if status === 'app_missing'}
        <ol class="steps">
          <li>
            {t('systeme.netatmo.app_etape1')}
            <a class="btn primary" href="https://dev.netatmo.com/apps/createanapp" target="_blank" rel="noopener noreferrer">{t('systeme.netatmo.ouvrir_dev')}</a>
          </li>
          <li>{t('systeme.netatmo.app_etape2')} <code>{redirect}</code></li>
          <li>{t('systeme.netatmo.app_etape3')}</li>
        </ol>
      {/if}
      <form class="form" onsubmit={(e) => (e.preventDefault(), saveKeys(a.instance))} autocomplete="off">
        <label>client ID<input bind:value={clientId} spellcheck="false" autocomplete="off" /></label>
        <label>client secret<input bind:value={clientSecret} type="password" spellcheck="false" autocomplete="new-password" /></label>
        <button class="btn primary" disabled={busy || !clientId.trim() || !clientSecret.trim()}>{busy ? '…' : t('systeme.netatmo.enregistrer')}</button>
        <small class="muted">{t('systeme.netatmo.code_demande')}</small>
      </form>
    {/if}

    {#if status === 'sign_in' && url}
      <ol class="steps">
        <li>
          {t('systeme.netatmo.etape1')}
          <a class="btn primary" href={url} target="_blank" rel="noopener noreferrer">{t('systeme.netatmo.ouvrir')}</a>
        </li>
        <li>{t('systeme.netatmo.etape2')}</li>
        <li>{t('systeme.netatmo.etape3', { adresse: redirect })}</li>
      </ol>
      <form class="form" onsubmit={(e) => (e.preventDefault(), connect(a.id))} autocomplete="off">
        <label for="netatmo-pasted">{t('systeme.netatmo.coller')}</label>
        <input id="netatmo-pasted" bind:value={pasted} placeholder={`${redirect}?state=…&code=…`} spellcheck="false" />
        <button class="btn primary" disabled={busy || pasted.trim().length < 8}>{busy ? t('systeme.netatmo.verification') : t('systeme.netatmo.connecter')}</button>
      </form>
      <small class="muted">{t('systeme.netatmo.aide')}</small>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </section>
{/each}

<style>
  .card {
    display: grid;
    gap: 14px;
    padding: 16px 18px;
    border-radius: var(--r-md);
    background: var(--warm-soft);
  }

  .card.done {
    background: var(--surface-2);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .ico {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface);
    flex: none;
  }

  .text {
    display: grid;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .steps {
    margin: 0;
    padding-left: 1.3em;
    display: grid;
    gap: 8px;
  }

  .steps li {
    overflow-wrap: anywhere;
  }

  .steps .btn {
    margin-left: 8px;
  }

  code {
    font-size: 0.9em;
    padding: 1px 6px;
    border-radius: 6px;
    background: var(--surface);
    user-select: all;
  }

  .form {
    display: grid;
    gap: 8px;
  }

  .form label {
    display: grid;
    gap: 4px;
  }

  .form input {
    font: inherit;
    padding: 9px 12px;
    border-radius: 10px;
    border: 1px solid var(--line, rgba(0, 0, 0, 0.15));
    background: var(--surface);
    color: inherit;
    min-width: 0;
  }

  .btn {
    display: inline-block;
    border: 0;
    border-radius: 999px;
    padding: 9px 16px;
    font: inherit;
    font-weight: 650;
    background: var(--surface);
    color: var(--ink);
    cursor: pointer;
    text-decoration: none;
    justify-self: start;
  }

  .btn.primary {
    background: var(--warm);
    color: #fff;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .btn:focus-visible,
  .form input:focus-visible {
    outline: 2px solid currentColor;
    outline-offset: 2px;
  }

  .error {
    margin: 0;
    color: var(--danger, #b3261e);
  }
</style>
