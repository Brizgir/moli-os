<script>
  import { onMount } from 'svelte';
  import { home, note } from '../lib/home.svelte.js';
  import { moli } from '../lib/moli.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import { playSpeech, stop, unlock } from '../lib/voice/player.js';
  import { fetchSpeech } from '../lib/voice/speech.js';
  import Icon from './Icon.svelte';

  /** Moli's settings: who thinks (Claude by Anthropic, or OpenAI) and with which model, the keys
   *  (given by a person, code asked; tested, filed encrypted in Moli, never shown again), today's
   *  cost and the voice. Anthropic has no voice: OpenAI's key gives it one, or Whisper and Piper. */
  let s = $state(null);
  /** Which key's form is open: `anthropic`, `openai` or none. */
  let open = $state(null);
  let key = $state('');
  let busy = $state(false);
  let playing = $state(null);

  const NAMES = { anthropic: 'Anthropic', openai: 'OpenAI' };
  // Where each provider's key is made: sign-up, credit, spending limit, keys, and how a key starts.
  const HOW = {
    anthropic: {
      signup: ['https://console.anthropic.com/', 'console.anthropic.com'],
      billing: ['https://console.anthropic.com/settings/billing', 'Billing'],
      limits: ['https://console.anthropic.com/settings/limits', 'Limits'],
      keys: ['https://console.anthropic.com/settings/keys', 'API keys'],
      step4: 'moli.reglages.anthropic_etape4',
      prefix: 'sk-ant-',
    },
    openai: {
      signup: ['https://platform.openai.com/signup', 'platform.openai.com'],
      billing: ['https://platform.openai.com/settings/organization/billing/overview', 'Billing'],
      limits: ['https://platform.openai.com/settings/organization/limits', 'Limits'],
      keys: ['https://platform.openai.com/api-keys', 'API keys'],
      step4: 'moli.reglages.etape4',
      prefix: 'sk-',
    },
  };
  const how = $derived(open ? HOW[open] : null);
  const models = $derived(s?.providers?.find((p) => p.id === s.provider)?.models ?? []);

  async function load() {
    try {
      const res = await fetch('/api/assistant/settings');
      if (res.ok) s = await res.json();
    } catch {
      /* the card stays hidden */
    }
  }
  onMount(load);

  async function saveKey() {
    busy = true;
    try {
      const res = await fetch('/api/assistant/key', {
        method: 'PUT',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ api_key: key.trim(), provider: open }),
      });
      if (res.status === 403) {
        // A person's gesture: the code first, then the same request again.
        home.held = { label: t('moli.reglages.cle_titre'), reason: 'ton code', custom: saveKey };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.error ?? res.statusText);
      key = '';
      open = null;
      note(t('moli.reglages.cle_enregistree'), 'info');
      moli.status = null;
      await Promise.all([load(), fetch('/api/assistant').then((r) => r.json()).then((j) => (moli.status = j))]);
    } catch (e) {
      note(t('moli.reglages.cle_erreur', { message: e.message.replace(/^invalid request: |^model unavailable: /, '') }), 'error');
    } finally {
      busy = false;
    }
  }

  /** Who thinks and with which model: it changes what Moli costs, so a person's code is asked. */
  async function think(change) {
    const before = s;
    const wanted = { provider: s.provider, ...change };
    s = { ...s, ...change };
    try {
      const res = await fetch('/api/assistant/brain', {
        method: 'PUT',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify(wanted),
      });
      if (res.status === 403) {
        s = before;
        home.held = { label: t('moli.reglages.modele_titre'), reason: 'ton code', custom: () => think(change) };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.error ?? res.statusText);
      s = body;
      note(t('moli.reglages.modele_choisi', { model: s.model }), 'info');
      moli.status = null;
      await fetch('/api/assistant').then((r) => r.json()).then((j) => (moli.status = j));
    } catch (e) {
      s = before;
      note(t('moli.reglages.modele_erreur', { message: e.message.replace(/^invalid request: /, '') }), 'error');
    }
  }

  const price = (n) => (n < 1 ? n.toFixed(2) : String(n));

  async function choose(change) {
    const before = s;
    s = { ...s, ...change };
    try {
      const res = await fetch('/api/assistant/settings', {
        method: 'PUT',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify(change),
      });
      if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error ?? res.statusText);
      s = await res.json();
    } catch (e) {
      s = before;
      note(t('moli.reglages.voix_erreur', { message: e.message }), 'error');
    }
  }

  /** Plays the sample in `voice` (the chosen tone applies). */
  async function listen(voice) {
    unlock();
    if (playing === voice) {
      stop();
      playing = null;
      return;
    }
    stop();
    playing = voice;
    try {
      await playSpeech(await fetchSpeech(t('moli.reglages.echantillon'), undefined, voice));
    } catch {
      note(t('moli.reglages.voix_muette'), 'error');
    } finally {
      if (playing === voice) playing = null;
    }
  }

  const label = (v) => v[0].toUpperCase() + v.slice(1);

  // A tone the house names in its language; the server's own label when this build does not know it.
  const toneName = (tone) => {
    const key = `moli.reglages.ton_${tone.id}`;
    const word = t(key);
    return word === key ? tone.label : word;
  };

  // A step of the key's how-to: the sentence is whole in the catalogue, `{link}` and `{code}` mark the
  // link to the provider and the key's start.
  const SPLIT = /(\{link\}|\{code\})/;
</script>

{#snippet step(sentence, [href, name], prefix)}
  {#each t(sentence).split(SPLIT) as piece, i (i)}
    {#if piece === '{link}'}<a {href} target="_blank" rel="noopener">{name}</a>{:else if piece === '{code}'}<code>{prefix}</code>{:else}{piece}{/if}
  {/each}
{/snippet}

{#if s}
  <section class="card" class:done={s.key || s.local_model}>
    <div class="row">
      <span class="ico"><Icon name={s.key || s.local_model ? 'talk' : 'lock'} size={18} /></span>
      <div class="text">
        <b>{t('moli.reglages.titre')}</b>
        <span class="muted">
          {#if s.local_model}
            {t('moli.reglages.local', { model: s.model })}
          {:else if s.key}
            {t('moli.reglages.cle_ok', { provider: NAMES[s.provider] })}
          {:else}
            {t('moli.reglages.cle_manque', { provider: NAMES[s.provider] })}
          {/if}
        </span>
      </div>
      {#if !s.local_model}
        <button class="btn" onclick={() => (open = open === s.provider ? null : s.provider)}>{s.key ? t('moli.reglages.remplacer') : t('moli.reglages.ajouter')}</button>
      {/if}
    </div>

    <div class="brain">
      <div class="head">
        <b>{t('moli.reglages.cerveau')}</b>
        <div class="tones" role="radiogroup" aria-label={t('moli.reglages.cerveau_aria')}>
          {#each s.providers ?? [] as p (p.id)}
            <button role="radio" aria-checked={s.provider === p.id} class:on={s.provider === p.id} onclick={() => s.provider !== p.id && think({ provider: p.id, model: null })}>{t(`moli.reglages.fournisseur_${p.id}`)}</button>
          {/each}
        </div>
      </div>
      {#if models.length > 1}
        <div class="tones models" role="radiogroup" aria-label={t('moli.reglages.modele_aria')}>
          {#each models as m (m)}
            <button role="radio" aria-checked={s.model === m} class:on={s.model === m} onclick={() => s.model !== m && think({ model: m })}>{m}</button>
          {/each}
        </div>
      {/if}
      <small class="muted">
        {#if s.rates}{t('moli.reglages.bareme', { model: s.model, input: price(s.rates.input), cached: price(s.rates.cached), output: price(s.rates.output) })}{/if}
        {#if s.today}{' '}{t('moli.reglages.aujourdhui', { n: s.today.exchanges, dollars: s.today.dollars.toFixed(2) })}{/if}
      </small>
    </div>

    {#if open && how}
      <div class="how">
        <b>{t('moli.reglages.comment', { provider: NAMES[open] })}</b>
        <ol>
          <li>{@render step('moli.reglages.etape1', how.signup, how.prefix)}</li>
          <li>{@render step('moli.reglages.etape2', how.billing, how.prefix)}</li>
          <li>{@render step('moli.reglages.etape3', how.limits, how.prefix)}</li>
          <li>{@render step(how.step4, how.keys, how.prefix)}</li>
          <li>{t('moli.reglages.etape5', { provider: NAMES[open] })}</li>
        </ol>
      </div>
      <form class="form" onsubmit={(e) => (e.preventDefault(), saveKey())} autocomplete="off">
        <label>{t('moli.reglages.cle_champ', { provider: NAMES[open] })}<input bind:value={key} type="password" placeholder={`${how.prefix}…`} spellcheck="false" autocomplete="new-password" /></label>
        <button class="btn primary" disabled={busy || key.trim().length < 20}>{busy ? t('moli.reglages.verification') : t('moli.reglages.enregistrer')}</button>
        <small class="muted">{t('moli.reglages.confidentialite')}</small>
      </form>
    {/if}

    {#if s.provider === 'anthropic'}
      <div class="row voice-key">
        <div class="text">
          <b>{t('moli.reglages.cle_voix')}</b>
          <span class="muted">
            {#if s.voice_cloud}
              {t('moli.reglages.voix_openai')}
            {:else if s.local_voice || s.local_listen}
              {t('moli.reglages.voix_locale')}
            {:else}
              {t('moli.reglages.voix_aucune')}
            {/if}
          </span>
        </div>
        <button class="btn" onclick={() => (open = open === 'openai' ? null : 'openai')}>{s.keys?.openai ? t('moli.reglages.remplacer') : t('moli.reglages.ajouter')}</button>
      </div>
    {/if}

    {#if s.voice_cloud && (s.key || s.local_model)}
      <div class="voice">
        <div class="head">
          <b>{t('moli.reglages.voix')}</b>
          <div class="tones" role="radiogroup" aria-label={t('moli.reglages.ton_aria')}>
            {#each s.styles as tone (tone.id)}
              <button role="radio" aria-checked={s.style === tone.id} class:on={s.style === tone.id} onclick={() => choose({ style: tone.id })}>{toneName(tone)}</button>
            {/each}
          </div>
        </div>
        <div class="voices" role="radiogroup" aria-label={t('moli.reglages.voix_aria')}>
          {#each s.voices as v (v)}
            <div class="voice-chip" class:on={s.voice === v}>
              <button class="pick" role="radio" aria-checked={s.voice === v} onclick={() => choose({ voice: v })}>{label(v)}</button>
              <button class="hear" onclick={() => listen(v)} aria-label={t('moli.reglages.ecouter_voix', { voice: label(v) })} title={t('moli.reglages.ecouter')}>
                <Icon name={playing === v ? 'stop' : 'play'} size={16} />
              </button>
            </div>
          {/each}
        </div>
        {#if s.paces}
          <div class="head">
            <b>{t('moli.reglages.debit')}</b>
            <div class="tones" role="radiogroup" aria-label={t('moli.reglages.debit_aria')}>
              {#each s.paces as pace (pace.id)}
                <button role="radio" aria-checked={s.pace === pace.id} class:on={s.pace === pace.id} onclick={() => choose({ pace: pace.id })}>{pace.label}</button>
              {/each}
            </div>
          </div>
        {/if}
        <small class="muted">{t('moli.reglages.aide_voix')}</small>
      </div>
    {/if}
  </section>
{/if}

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
    gap: var(--sp-3);
    flex-wrap: wrap;
  }

  .ico {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface);
    color: var(--warm-ink);
  }

  .done .ico {
    color: var(--good);
  }

  .text {
    display: grid;
    gap: 2px;
    flex: 1;
    min-width: 220px;
  }

  .text .muted,
  small {
    font-size: var(--fs-sm);
  }

  .btn {
    border: 0;
    border-radius: 999px;
    padding: 9px 16px;
    font: inherit;
    font-weight: 650;
    background: var(--surface);
    color: var(--ink);
    cursor: pointer;
  }

  .btn.primary {
    background: var(--warm);
    color: #fff;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .brain {
    display: grid;
    gap: var(--sp-2);
  }

  .models {
    flex-wrap: wrap;
    border-radius: var(--r-sm);
  }

  .voice-key .btn {
    background: var(--surface);
  }

  .how {
    display: grid;
    gap: 6px;
    padding: 12px 14px;
    border-radius: var(--r-sm);
    background: var(--surface);
    font-size: var(--fs-md);
  }

  .how ol {
    margin: 0;
    padding-left: 20px;
    display: grid;
    gap: var(--sp-1);
    color: var(--ink-2);
  }

  .how a {
    color: var(--ink);
    font-weight: 650;
  }

  .form {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 220px), 1fr));
    gap: 10px 14px;
    align-items: end;
  }

  label {
    display: grid;
    gap: var(--sp-1);
    font-size: var(--fs-xs);
    font-weight: 650;
    color: var(--ink-2);
  }

  input {
    font: inherit;
    font-size: var(--fs-md);
    padding: 9px 12px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
  }

  .form small {
    grid-column: 1 / -1;
  }

  .voice {
    display: grid;
    gap: 10px;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }

  .tones {
    display: flex;
    gap: var(--sp-1);
    padding: 3px;
    border-radius: 999px;
    background: var(--surface);
  }

  .tones button {
    border: 0;
    border-radius: 999px;
    padding: 6px 12px;
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 650;
    background: none;
    color: var(--ink-2);
    cursor: pointer;
  }

  .tones button.on {
    background: var(--ink);
    color: var(--bg);
  }

  .voices {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(118px, 1fr));
    gap: var(--sp-2);
  }

  .voice-chip {
    display: flex;
    align-items: center;
    border-radius: 999px;
    background: var(--surface);
    box-shadow: inset 0 0 0 1px var(--line);
  }

  .voice-chip.on {
    box-shadow: inset 0 0 0 2px var(--ink);
  }

  .pick {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 650;
    color: var(--ink);
    padding: 8px 4px 8px 14px;
    text-align: left;
    cursor: pointer;
  }

  .hear {
    flex: none;
    width: 34px;
    height: 34px;
    margin: 2px;
    border: 0;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-2);
    color: var(--ink-2);
    cursor: pointer;
  }
</style>
