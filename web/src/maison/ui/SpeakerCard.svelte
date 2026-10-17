<script>
  import { home, device, value, act, num, nameOf, speakerState, speakerGroup } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** A speaker (Sonos, Echo…) or a group of speakers: what plays,
   *  play/pause, volume and mute when it has them, and « Mets… » /
   *  « Dis… » when it takes words (an Echo). `inline`: inside a room's
   *  card, under its pad. */
  let { id, name = null, inline = false } = $props();

  const d = $derived(device(id));
  const can = (key) => !!d?.points.some((p) => p.key === key && p.access?.write);
  const group = $derived(speakerGroup(d));
  const state = $derived(speakerState(id));
  const playing = $derived(state === 'playing');
  const title = $derived(value(id, 'title'));
  const artist = $derived(value(id, 'artist'));
  const volume = $derived(value(id, 'volume'));
  const muted = $derived(value(id, 'mute') === true);
  // Only the sound bar beside the TV plays « the TV's sound » without a title.
  const soundBar = $derived(id === home.config?.salon?.speaker);
  let dragging = $state(null);

  /** What the field does: put on music (`play`) or say a sentence (`say`). */
  const modes = $derived(['play', 'say'].filter(can));
  let mode = $state('play');
  const current = $derived(modes.includes(mode) ? mode : modes[0]);
  let words = $state('');
  let sending = $state(false);

  async function send(e) {
    e.preventDefault();
    const text = words.trim();
    if (!text || sending) return;
    sending = true;
    const label = t(current === 'say' ? 'commun.enceinte.label_dire' : 'commun.enceinte.label_mettre', { name: nameOf(id, name) });
    if ((await act(`${id}/${current}`, text, label)) !== 'error') words = '';
    sending = false;
  }
</script>

<section class:card={!inline} class:inline class="sound">
  <div class="card-head">
    <h2><Icon name={group ? 'speaker-multiple' : 'speaker'} size={18} />{nameOf(id, name)}</h2>
    <span class="chip {playing ? 'warm' : ''}">{t(`commun.enceinte.${state}`)}</span>
  </div>
  {#if title}
    <p class="track"><b>{title}</b>{#if artist}<span class="muted">{` · ${artist}`}</span>{/if}</p>
  {:else}
    <p class="muted">{playing ? t(soundBar ? 'commun.enceinte.son_tele' : 'commun.enceinte.joue') : t(state === 'unknown' ? 'commun.enceinte.inconnu' : 'commun.enceinte.rien')}</p>
  {/if}
  {#if group}<p class="muted small">{t('commun.enceinte.groupe')}</p>{/if}
  <div class="row" class:alone={!can('volume')}>
    <button class="round" onclick={() => act(`${id}/playing`, !playing, t(playing ? 'commun.enceinte.pause' : 'commun.enceinte.lecture'))} aria-label={t(playing ? 'commun.enceinte.pause' : 'commun.enceinte.lecture')}>
      <Icon name={playing ? 'pause' : 'play'} size={26} />
    </button>
    {#if can('volume')}
      <div class="vol">
        <Icon name="volume-minus" size={18} />
        <input
          type="range"
          min="0"
          max="100"
          value={dragging ?? volume ?? 0}
          oninput={(e) => (dragging = Number(e.currentTarget.value))}
          onchange={(e) => {
            const v = Number(e.currentTarget.value);
            dragging = null;
            act(`${id}/volume`, v, t('commun.enceinte.volume_de', { name: nameOf(id, name) }));
          }}
          aria-label={t('commun.enceinte.volume')} />
        <Icon name="volume-plus" size={18} />
        <b class="num">{num(dragging ?? volume)}</b>
      </div>
    {/if}
    {#if can('mute')}
      <button class="round soft" class:active={muted} onclick={() => act(`${id}/mute`, !muted, t('commun.enceinte.sourdine'))} aria-label={t('commun.enceinte.couper_son')} aria-pressed={muted}>
        <Icon name={muted ? 'volume-off' : 'volume-high'} size={22} />
      </button>
    {/if}
  </div>
  {#if modes.length}
    <form class="ask" onsubmit={send}>
      {#if modes.length > 1}
        <div class="modes" role="radiogroup" aria-label={t('commun.enceinte.que_faire')}>
          {#each modes as m (m)}
            <button type="button" role="radio" aria-checked={current === m} class:active={current === m} onclick={() => (mode = m)}>
              <Icon name={m === 'say' ? 'talk' : 'play-circle'} size={18} />{t(m === 'say' ? 'commun.enceinte.dire' : 'commun.enceinte.mettre')}
            </button>
          {/each}
        </div>
      {/if}
      <div class="field">
        <input
          type="text"
          bind:value={words}
          maxlength={current === 'say' ? 250 : 200}
          placeholder={t(current === 'say' ? 'commun.enceinte.dire_exemple' : 'commun.enceinte.mettre_exemple')}
          aria-label={t(current === 'say' ? 'commun.enceinte.dire' : 'commun.enceinte.mettre')} />
        <button type="submit" class="go" disabled={!words.trim() || sending} aria-label={t('commun.enceinte.envoyer')}>
          <Icon name="send" size={20} />
        </button>
      </div>
    </form>
  {/if}
</section>

<style>
  .inline {
    padding: 14px;
    border-radius: var(--r-md);
    background: var(--surface-2);
  }

  .track b {
    font-weight: 650;
  }

  .small {
    font-size: 12.5px;
    margin-top: 4px;
  }

  .row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 14px;
    margin-top: 14px;
  }

  .row.alone {
    grid-template-columns: auto 1fr;
  }

  .round {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    border: 0;
    background: var(--ink);
    color: var(--bg);
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .round.soft {
    width: 48px;
    height: 48px;
    background: var(--surface-2);
    color: var(--ink-2);
  }

  .inline .round.soft {
    background: var(--surface);
  }

  .round.soft.active {
    color: var(--alert);
  }

  .vol {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--ink-3);
    min-width: 0;
  }

  .vol input {
    flex: 1;
    min-width: 0;
    accent-color: var(--ink);
    height: 28px;
  }

  .vol b {
    width: 28px;
    text-align: right;
    color: var(--ink);
  }

  /* « Mets… » / « Dis… »: one field, big enough for a finger. */
  .ask {
    display: grid;
    gap: 10px;
    margin-top: 14px;
  }

  .modes {
    display: flex;
    gap: 8px;
  }

  .modes button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 44px;
    border: 0;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--ink-2);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }

  .inline .modes button {
    background: var(--surface);
  }

  .modes button.active {
    background: var(--ink);
    color: var(--bg);
  }

  .field {
    display: flex;
    gap: 8px;
  }

  .field input {
    flex: 1;
    min-width: 0;
    min-height: 48px;
    padding: 0 16px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 15px;
  }

  .go {
    flex: none;
    width: 48px;
    height: 48px;
    border: 0;
    border-radius: 50%;
    background: var(--warm);
    color: #fff;
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .go:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
