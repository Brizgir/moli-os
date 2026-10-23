<script>
  import { home, note, justified, savePages, pageChoices, camerasAsDoorbell, PAGE_IDS, PAGE_CHOICES } from '../lib/home.svelte.js';
  import { menuFacts } from '../lib/menu.svelte.js';
  import { asHuman, CANCELLED } from '../lib/auto.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** « Pages affichées »: each page of the menu shows when the house has
   *  what it serves (automatic), or always, or never. Kept in `home.json`
   *  (`pages`), as a person: the code first. */
  const NAMES = {
    pieces: 'commun.nav.pieces',
    chauffage: 'commun.nav.chauffage',
    salon: 'commun.nav.salon',
    plan: 'commun.nav.plan',
    carte: 'commun.nav.carte',
    cameras: 'commun.nav.cameras',
    parler: 'commun.nav.parler',
    energie: 'commun.nav.energie',
    solaire: 'commun.nav.solaire',
    impression: 'commun.nav.impression',
    dehors: 'commun.nav.dehors',
    automatismes: 'commun.nav.automatismes',
    infra: 'commun.nav.infra',
    moli: 'commun.demander_moli',
  };
  const ALWAYS_USEFUL = ['pieces', 'automatismes', 'moli'];

  const auto = $derived(justified(menuFacts()));
  const choices = $derived(pageChoices());
  const shownCount = $derived(PAGE_IDS.filter((id) => (choices[id] === 'always' ? true : choices[id] === 'never' ? false : auto[id])).length);
  let open = $state(false);
  let busy = $state(null);

  const nameOf = (id) => t(id === 'cameras' && camerasAsDoorbell() ? 'commun.nav.sonnette_cameras' : NAMES[id]);

  /** Why the automatic choice is what it is, in a few words. */
  function why(id) {
    if (ALWAYS_USEFUL.includes(id)) return t('systeme.pages.toujours_utile');
    if (id === 'infra') return t('systeme.pages.atelier');
    return t(`systeme.pages.${auto[id] ? 'oui' : 'non'}.${id}`);
  }

  async function choose(id, choice) {
    if ((choices[id] ?? 'auto') === choice) return;
    busy = id;
    try {
      await asHuman(t('systeme.pages.titre'), () => savePages({ [id]: choice }), 'reglage');
    } catch (err) {
      if (err.message !== CANCELLED) note(err.message, 'error');
    } finally {
      busy = null;
    }
  }
</script>

{#if home.config}
  <section class="card pages">
    <div class="head">
      <span class="icon"><Icon name="light-group" size={20} /></span>
      <div class="what">
        <b>{t('systeme.pages.titre')}</b>
        <span class="muted">{t('systeme.pages.resume', { shown: shownCount, count: PAGE_IDS.length })}</span>
      </div>
      <button class="toggle" onclick={() => (open = !open)} aria-expanded={open}>{open ? t('systeme.pages.fermer') : t('systeme.pages.choisir')}</button>
    </div>
    {#if open}
      <p class="muted intro">{t('systeme.pages.explication')}</p>
      <ul>
        {#each PAGE_IDS as id (id)}
          {@const choice = choices[id] ?? 'auto'}
          <li>
            <div class="name">
              <b>{nameOf(id)}</b>
              <small class="muted">{why(id)}</small>
            </div>
            <div class="choice" role="radiogroup" aria-label={nameOf(id)}>
              {#each PAGE_CHOICES as c (c)}
                <button role="radio" aria-checked={choice === c} class:on={choice === c} disabled={busy === id} onclick={() => choose(id, c)}>{t(`systeme.pages.choix.${c}`)}</button>
              {/each}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .pages {
    display: grid;
    gap: 14px;
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 16px;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 42px;
    height: 42px;
    border-radius: 50%;
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  .what {
    display: grid;
    flex: 1;
    min-width: 200px;
    gap: 2px;
    font-size: var(--fs-md);
  }

  .toggle {
    min-height: 44px;
    padding: 0 18px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .intro {
    margin: 0;
    font-size: var(--fs-md);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 6px;
  }

  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 16px;
    padding: 10px 12px;
    border-radius: var(--r-sm);
    background: var(--surface-2);
  }

  .name {
    display: grid;
    gap: 2px;
    min-width: 180px;
    flex: 1;
  }

  .name small {
    font-size: 12.5px;
  }

  .choice {
    display: inline-flex;
    padding: 3px;
    border-radius: 999px;
    background: var(--surface);
  }

  .choice button {
    min-height: 44px;
    padding: 0 14px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--ink-3);
    font: inherit;
    font-size: 13.5px;
    font-weight: 650;
    cursor: pointer;
  }

  .choice button.on {
    background: var(--ink);
    color: var(--surface);
  }

  .choice button:disabled {
    cursor: progress;
  }
</style>
