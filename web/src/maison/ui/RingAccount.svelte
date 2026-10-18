<script>
  import { hub, home, value } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** Each Ring account Moli reads: connected (its devices), or the sign-in.
   *  Ring has no sign-in page for other apps: the person makes a token on
   *  their own computer (Ring's own client asks the e-mail, password and
   *  code there) and pastes it here. Moli never sees the password; the
   *  pasted token goes to Ring once, is never stored, journaled or shown
   *  (the point is « secret »); only the token Ring gives back is kept, in
   *  Moli's encrypted store. */
  const accounts = $derived(
    Object.values(hub.devices ?? {}).filter((d) => d.manufacturer === 'Ring' && d.model === 'Ring' && d.points?.some((p) => p.key === 'sign_in')),
  );
  const devicesOf = (account) =>
    Object.values(hub.devices ?? {}).filter((d) => d.instance === account.instance && d.manufacturer === 'Ring' && d.model !== 'Ring').length;

  let pasted = $state('');
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
        // A person's code first; the same step is sent again after it.
        home.held = { label: t('systeme.ring.connecter'), reason: 'ton code', custom: () => connect(id) };
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
</script>

{#each accounts as a (a.id)}
  {@const status = value(a.id, 'status')}
  <section class="card" class:done={status === 'connected'}>
    <div class="row">
      <span class="ico"><Icon name="bell" size={18} /></span>
      <div class="text">
        <b>{t('systeme.ring.titre')}</b>
        <span class="muted">
          {#if status === 'connected'}
            {t('systeme.ring.connecte', { n: devicesOf(a) })}
          {:else if status === 'sign_in'}
            {t('systeme.ring.a_connecter')}
          {:else if status === 'slow_down'}
            {t('systeme.ring.attendre')}
          {:else if status === 'error'}
            {t('systeme.ring.erreur')}
          {:else}
            {t('systeme.ring.connexion')}
          {/if}
        </span>
      </div>
    </div>

    {#if status === 'connected'}
      {@const push = value(a.id, 'push')}
      <p class="push" class:on={push === 'active'}>
        {#if push === 'active'}
          {t('systeme.ring.alertes_actives')}
        {:else if push === 'off'}
          {t('systeme.ring.alertes_off')}
        {:else}
          {t('systeme.ring.alertes_repli')}
        {/if}
      </p>
    {/if}

    {#if status === 'sign_in'}
      <ol class="steps">
        <li>{t('systeme.ring.etape1')} <code>npx -p ring-client-api ring-auth-cli</code></li>
        <li>{t('systeme.ring.etape2')}</li>
        <li>{t('systeme.ring.etape3')}</li>
      </ol>
      <form class="form" onsubmit={(e) => (e.preventDefault(), connect(a.id))} autocomplete="off">
        <label for="ring-pasted">{t('systeme.ring.coller')}</label>
        <input id="ring-pasted" type="password" bind:value={pasted} autocomplete="off" spellcheck="false" />
        <button class="btn primary" disabled={busy || pasted.trim().length < 16}>{busy ? t('systeme.ring.verification') : t('systeme.ring.connecter')}</button>
        {#if error}<p class="error">{error}</p>{/if}
      </form>
      <small class="muted">{t('systeme.ring.aide')}</small>
    {/if}
    <small class="muted">{t('systeme.ring.officieux')}</small>
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
  }

  .text {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .push {
    margin: 0;
    color: var(--ink-2);
  }

  .push.on {
    color: inherit;
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

  .steps code {
    font-size: 0.92em;
    padding: 1px 6px;
    border-radius: 6px;
    background: var(--surface);
  }

  .form {
    display: grid;
    gap: 8px;
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
