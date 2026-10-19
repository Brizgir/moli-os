<script>
  import Icon from '../ui/Icon.svelte';
  import { note } from '../lib/home.svelte.js';
  import { api, asHuman, CANCELLED } from '../lib/auto.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';

  /** What Moli proposed about an existing automation (change it, switch it
   *  off or on, delete it, run it). Nothing has moved yet: a person confirms
   *  here, with the house code when there is no session, or says no.
   *  `before`: what the automation does now. `ondone(result)`: after a
   *  confirmation or a refusal (`result.deleted` when it is gone). */
  let { proposal, before = '', ondone = () => {} } = $props();

  let busy = $state(false);

  const kind = $derived(proposal.kind);
  const action = $derived(t('automatismes.proposition.confirmer_' + kind));

  async function confirm() {
    busy = true;
    try {
      const done = await asHuman(t('automatismes.proposition.label', { action, nom: proposal.name }), () => api.confirm(proposal.id));
      note(done?.deleted ? t('automatismes.editeur.supprime') : t('automatismes.proposition.faite'));
      ondone(done ?? {});
    } catch (err) {
      if (err.status === 404) {
        note(t('automatismes.proposition.expiree'), 'error');
        ondone({});
      } else if (err.status === 409) {
        note(t('automatismes.carte.change_entre_temps'), 'error');
        ondone({});
      } else if (err.message !== CANCELLED) note(err.message, 'error');
    } finally {
      busy = false;
    }
  }

  async function refuse() {
    busy = true;
    try {
      await api.dismiss(proposal.id);
      note(t('automatismes.proposition.refusee'));
    } catch {
      /* gone already: nothing to undo */
    } finally {
      busy = false;
      ondone({});
    }
  }
</script>

<div class="proposal" class:danger={kind === 'delete'}>
  <p class="title"><Icon name="sparkles" size={16} />{t('automatismes.proposition.titre_' + kind)}</p>
  {#if kind === 'update'}
    {#if before}<p class="was">{t('automatismes.proposition.avant', { texte: before })}</p>{/if}
    <p class="now">{t('automatismes.proposition.apres', { texte: proposal.summary })}</p>
  {/if}
  {#if proposal.protected?.length && kind !== 'delete' && kind !== 'pause'}
    <p class="lock"><Icon name="lock" size={14} />{t('automatismes.carte.agit_dans', { pieces: proposal.protected.join(', ') })}</p>
  {/if}
  <p class="muted small">{t('automatismes.proposition.rien_avant')}</p>
  <div class="row">
    <button class="primary" onclick={confirm} disabled={busy}><Icon name="shield" size={16} />{action}</button>
    <button class="ghost" onclick={refuse} disabled={busy}>{t('automatismes.proposition.refuser')}</button>
  </div>
</div>

<style>
  .proposal {
    display: grid;
    gap: 8px;
    padding: 12px 14px;
    border-radius: 14px;
    background: var(--cool-soft);
  }

  .proposal.danger {
    background: var(--alert-soft);
  }

  .title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 700;
  }

  .was {
    font-size: 13px;
    color: var(--ink-3);
    text-decoration: line-through;
  }

  .now {
    font-size: 15px;
    line-height: 1.45;
  }

  .lock {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--cool);
    font-weight: 600;
  }

  .small {
    font-size: 12px;
  }

  .row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .primary,
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    border-radius: 999px;
    padding: 9px 16px;
    font-weight: 700;
  }

  .primary {
    background: var(--ink);
    color: var(--bg);
  }

  .primary:disabled,
  .ghost:disabled {
    opacity: 0.4;
  }

  .ghost {
    background: var(--surface);
    color: var(--ink);
  }
</style>
