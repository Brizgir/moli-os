<script>
  import { hub, act, nameOf, hidden, deviceKind, speakerState, speakerGroup, value } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** The music at a glance: the speaker that plays (else the one paused,
   *  else the first), play/pause in one tap, and how many there are.
   *  `device`: a chosen speaker instead (the home page's arrangement);
   *  `title`: the person's own. */
  let { device = null, title: heading = null } = $props();

  const speakers = $derived(Object.values(hub.devices).filter((d) => !hidden(d.id) && deviceKind(d) === 'speaker'));
  const singles = $derived(speakers.filter((d) => !speakerGroup(d)));
  const groups = $derived(speakers.filter(speakerGroup));
  const rank = { playing: 0, paused: 1, unknown: 2, stopped: 3 };
  const main = $derived(
    (device && speakers.find((d) => d.id === device)) ?? [...singles].sort((a, b) => rank[speakerState(a.id)] - rank[speakerState(b.id)])[0] ?? null,
  );
  const state = $derived(main ? speakerState(main.id) : 'stopped');
  const playing = $derived(state === 'playing');
  const title = $derived(main ? value(main.id, 'title') : null);
  const canPlay = $derived(!!main?.points.some((p) => p.key === 'playing' && p.access?.write));
</script>

{#if main}
  <section class="card music">
    <div class="card-head">
      <h2><Icon name="speaker" size={18} />{heading ?? t('maison.accueil.musique')}</h2>
      <a class="more" href="#/pieces" aria-label={t('maison.accueil.toutes_pieces')}><Icon name="arrow-top-right" size={18} /></a>
    </div>
    <div class="now">
      {#if canPlay}
        <button class="play" onclick={() => act(`${main.id}/playing`, !playing, t(playing ? 'commun.enceinte.pause' : 'commun.enceinte.lecture'))} aria-label={t(playing ? 'commun.enceinte.pause' : 'commun.enceinte.lecture')}>
          <Icon name={playing ? 'pause' : 'play'} size={20} />
        </button>
      {/if}
      <div class="what">
        <b>{nameOf(main.id)}</b>
        <span class="muted">{t(`commun.enceinte.${state}`)}{title ? ` · ${title}` : ''}</span>
      </div>
    </div>
    <span class="muted small">
      {t('maison.accueil.enceintes', { count: singles.length })}{groups.length ? ` · ${t('maison.accueil.groupes', { count: groups.length, names: groups.map((g) => nameOf(g.id)).join(', ') })}` : ''}
    </span>
  </section>
{/if}

<style>
  .music {
    display: grid;
    gap: 14px;
  }

  .now {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border-radius: 16px;
    background: var(--surface-2);
  }

  .play {
    flex: none;
    width: 46px;
    height: 46px;
    border: 0;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--ink);
    color: var(--surface);
    cursor: pointer;
  }

  .what {
    display: grid;
    min-width: 0;
  }

  .what b,
  .what span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .what span,
  .small {
    font-size: 13px;
  }
</style>
