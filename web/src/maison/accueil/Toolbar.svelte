<script>
  import { dashboard, undo, cancelEditing, finishEditing, forgetLayouts, beginEditing, draftSet } from '../lib/dashboard.svelte.js';
  import { addLayout, removeLayout, names } from '../lib/layout.js';
  import { note } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** The dark bar over the page while it is being changed: whose
   *  arrangement, the named ones, undo, the default, the catalogue, done. */
  let { ondefault, oncatalogue } = $props();

  const e = $derived(dashboard.editing);
  const list = $derived(names(e?.set));
  const screenName = $derived(t(dashboard.screen === 'phone' ? 'maison.modifier.ecran_telephone' : 'maison.modifier.ecran_grand'));
  const both = $derived(dashboard.canEditHouse && dashboard.canEditMine);
  const dirty = $derived((e?.history.length ?? 0) > 0);

  let naming = $state(false);
  let newName = $state('');
  let forgetting = $state(false);

  function pick(key) {
    e.key = key;
  }

  function addNamed(ev) {
    ev.preventDefault();
    const { set, key } = addLayout(e.set, newName, e.key);
    if (!key) {
      note(t('maison.modifier.nom_refuse'), 'error');
      return;
    }
    draftSet(set, key);
    naming = false;
    newName = '';
  }

  function removeNamed() {
    if (e.key === 'default') return;
    draftSet(removeLayout(e.set, e.key), 'default');
  }

  async function done() {
    if (await finishEditing()) note(t('maison.modifier.enregistree'));
  }

  async function forget() {
    if (await forgetLayouts()) note(t('maison.modifier.oubliee'));
    forgetting = false;
  }
</script>

<div class="bar" role="toolbar" aria-label={t('maison.modifier.titre')}>
  <div class="row">
    <span class="what"><Icon name="pencil" size={18} /><b>{t('maison.modifier.titre')}</b></span>
    <span class="scope">
      {t('maison.modifier.disposition', { screen: screenName })}
      {#if both}
        <button class="chip" class:on={e.scope === 'mine'} disabled={dirty} onclick={() => beginEditing('mine')}>{t('maison.modifier.la_mienne')}</button>
        <button class="chip" class:on={e.scope === 'house'} disabled={dirty} onclick={() => beginEditing('house')}>{t('maison.modifier.la_maison')}</button>
      {:else}
        <b>{t(e.scope === 'house' ? 'maison.modifier.la_maison' : 'maison.modifier.la_mienne')}</b>
      {/if}
    </span>
    <span class="grow"></span>
    <button class="action" onclick={cancelEditing}><Icon name="close" size={16} />{t('maison.modifier.abandonner')}</button>
    <button class="action primary" disabled={e.busy} onclick={done}><Icon name="ok" size={16} />{t('maison.modifier.termine')}</button>
  </div>
  <div class="row">
    <div class="named" role="radiogroup" aria-label={t('maison.modifier.nommees')}>
      {#each list as l (l.key)}
        <button role="radio" aria-checked={e.key === l.key} class="chip" class:on={e.key === l.key} onclick={() => pick(l.key)}>{l.name ?? t('maison.modifier.normale')}</button>
      {/each}
      {#if naming}
        <form class="naming" onsubmit={addNamed}>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={newName} maxlength="60" placeholder={t('maison.modifier.nom_exemple')} autofocus />
          <button type="submit" class="chip on">{t('maison.modifier.creer')}</button>
          <button type="button" class="chip" onclick={() => (naming = false)}>{t('maison.modifier.annuler_nom')}</button>
        </form>
      {:else}
        <button class="chip add" aria-label={t('maison.modifier.nouvelle_nommee')} onclick={() => (naming = true)}><Icon name="plus" size={16} /></button>
        {#if e.key !== 'default'}
          <button class="chip" onclick={removeNamed}><Icon name="trash" size={14} />{t('maison.modifier.retirer_nommee')}</button>
        {/if}
      {/if}
    </div>
    <span class="grow"></span>
    <button class="action" disabled={!dirty} onclick={undo}><Icon name="undo" size={16} />{t('maison.modifier.annuler')}</button>
    <button class="action" onclick={() => ondefault?.()}><Icon name="restore" size={16} />{t('maison.modifier.par_defaut')}</button>
    <button class="action" onclick={() => oncatalogue?.()}><Icon name="plus" size={16} />{t('maison.modifier.ajouter')}</button>
  </div>
  {#if e.error}<p class="error" role="alert">{e.error}</p>{/if}
  <p class="help">
    {t('maison.modifier.aide')}
    {#if (e.scope === 'house' ? dashboard.house : dashboard.mine)}
      {#if forgetting}
        <button class="link" onclick={forget}>{t('maison.modifier.oublier_confirmer')}</button>
        <button class="link" onclick={() => (forgetting = false)}>{t('maison.modifier.annuler_nom')}</button>
      {:else}
        <button class="link" onclick={() => (forgetting = true)}>{t('maison.modifier.oublier')}</button>
      {/if}
    {/if}
  </p>
</div>

<style>
  .bar {
    position: sticky;
    top: 8px;
    z-index: 5;
    display: grid;
    gap: 10px;
    padding: 12px 16px;
    border-radius: 20px;
    background: #171c28;
    color: #eceff5;
    box-shadow: var(--shadow-lift);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .grow {
    flex: 1;
  }

  .what {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 15px;
  }

  .scope {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: #b3bac8;
  }

  .scope b {
    color: #eceff5;
  }

  .named {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  button {
    font: inherit;
    cursor: pointer;
    border: 0;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 36px;
    padding: 0 14px;
    border-radius: 999px;
    background: #232935;
    color: #eceff5;
    font-size: 13px;
    font-weight: 650;
  }

  .chip.on {
    background: #f5ae4c;
    color: #33270f;
  }

  .chip:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .chip.add {
    width: 36px;
    padding: 0;
    justify-content: center;
    border: 1px dashed #788195;
    background: transparent;
  }

  .naming {
    display: inline-flex;
    gap: 6px;
    align-items: center;
  }

  .naming input {
    font: inherit;
    font-size: 14px;
    min-height: 36px;
    padding: 0 12px;
    border-radius: 10px;
    border: 1px solid #3a4254;
    background: #0d1016;
    color: #eceff5;
    width: 160px;
  }

  .action {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    padding: 0 14px;
    border-radius: 12px;
    background: #232935;
    color: #eceff5;
    font-size: 14px;
    font-weight: 600;
  }

  .action:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .action.primary {
    background: #f5ae4c;
    color: #33270f;
    font-weight: 700;
  }

  .help {
    margin: 0;
    font-size: 12.5px;
    color: #b3bac8;
  }

  .link {
    background: transparent;
    color: #f5ae4c;
    padding: 0 4px;
    min-height: 32px;
    font-size: 12.5px;
    font-weight: 650;
    text-decoration: underline;
  }

  .error {
    margin: 0;
    color: #ff7b62;
    font-size: 14px;
  }

  @media (max-width: 767.98px) {
    .bar {
      top: 4px;
      padding: 10px 12px;
    }

    .action {
      padding: 0 10px;
      font-size: 13px;
    }

    .help {
      display: none;
    }
  }
</style>
