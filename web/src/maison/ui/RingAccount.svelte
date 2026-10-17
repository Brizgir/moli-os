<script>
  import { hub, home, value } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** Each Ring account Moli reads: connected (its devices), or the two
   *  steps of the sign-in. The e-mail and password go from here to Moli,
   *  which hands them to Ring and forgets them: never stored, never in the
   *  journal (the points are « secret »); only Ring's token is kept, in
   *  Moli's encrypted store. */
  const accounts = $derived(
    Object.values(hub.devices ?? {}).filter((d) => d.manufacturer === 'Ring' && d.model === 'Ring' && d.points?.some((p) => p.key === 'sign_in')),
  );
  const devicesOf = (account) =>
    Object.values(hub.devices ?? {}).filter((d) => d.instance === account.instance && d.manufacturer === 'Ring' && d.model !== 'Ring').length;

  let email = $state('');
  let password = $state('');
  let code = $state('');
  let busy = $state(false);
  let error = $state('');

  async function send(id, key, text, label, done) {
    busy = true;
    error = '';
    try {
      const res = await fetch('/api/command', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ point: `${id}/${key}`, value: text }),
      });
      if (res.status === 403) {
        // A person's code first; the same step is sent again after it.
        home.held = { label, reason: 'ton code', custom: () => send(id, key, text, label, done) };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        error = body.error ?? res.statusText;
        return;
      }
      done();
    } catch (e) {
      error = e.message;
    } finally {
      busy = false;
    }
  }

  const signIn = (id) =>
    send(id, 'sign_in', JSON.stringify({ email: email.trim(), password }), t('systeme.ring.connecter'), () => {
      password = '';
    });

  const confirm = (id) =>
    send(id, 'code', code.trim(), t('systeme.ring.valider'), () => {
      code = '';
      email = '';
    });

  const byText = (by) =>
    by === 'totp' ? t('systeme.ring.par_appli') : by === 'email' ? t('systeme.ring.par_email') : t('systeme.ring.par_sms');
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
          {:else if status === 'code'}
            {byText(value(a.id, 'code_by'))}
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

    {#if status === 'sign_in'}
      <form class="form" onsubmit={(e) => (e.preventDefault(), signIn(a.id))}>
        <label for="ring-email">{t('systeme.ring.email')}</label>
        <input id="ring-email" type="email" bind:value={email} autocomplete="username" spellcheck="false" />
        <label for="ring-password">{t('systeme.ring.mot_de_passe')}</label>
        <input id="ring-password" type="password" bind:value={password} autocomplete="current-password" />
        <button class="btn primary" disabled={busy || !email.includes('@') || !password}>{busy ? t('systeme.ring.verification') : t('systeme.ring.connecter')}</button>
        {#if error}<p class="error">{error}</p>{/if}
      </form>
      <small class="muted">{t('systeme.ring.aide')}</small>
    {:else if status === 'code'}
      <form class="form" onsubmit={(e) => (e.preventDefault(), confirm(a.id))} autocomplete="off">
        <label for="ring-code">{t('systeme.ring.code')}</label>
        <input id="ring-code" bind:value={code} inputmode="numeric" autocomplete="one-time-code" spellcheck="false" />
        <button class="btn primary" disabled={busy || code.trim().length < 4}>{busy ? t('systeme.ring.verification') : t('systeme.ring.valider')}</button>
        {#if error}<p class="error">{error}</p>{/if}
      </form>
      <small class="muted">{t('systeme.ring.code_aide')}</small>
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
