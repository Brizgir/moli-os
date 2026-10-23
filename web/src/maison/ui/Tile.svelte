<script>
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** The one tile for a thing in the house, drawn as Apple Home draws
   *  them: an icon in a disc, a name, a word of state. Lit when `on` (the
   *  disc takes the tone: warm for a light, cool for the climate, good for
   *  what plays, alert for what is wrong), muted otherwise; `tint`: a
   *  lamp's own colour on the disc; `glyph`: a letter in the disc instead
   *  of the icon (a person). `value`: a figure that matters (a
   *  temperature), big. `badge`: a small icon top right (a lock, a hand).
   *  A tap is the main action (`onclick`, `href` for a link, `static` for
   *  a reading only); `onmore` adds a small round button top right
   *  (dimmer, remote…). `row`: the disc and the words side by side, low.
   *  `compact`: the disc alone, the name for the pointer and the reader.
   *  `pressed` / `expanded`: what the tap means, for ARIA. */
  let {
    icon = 'light',
    label,
    state = '',
    value = null,
    on = false,
    tone = 'warm',
    tint = null,
    glyph = null,
    badge = null,
    busy = false,
    dim = false,
    compact = false,
    row = false,
    open = false,
    pressed = null,
    expanded = null,
    onclick = null,
    href = null,
    static: fixed = false,
    onmore = null,
    moreIcon = 'tune',
    moreLabel = '',
    moreOpen = false,
  } = $props();

  // Dark words on a light tint, white ones on a deep one.
  const tintInk = $derived.by(() => {
    if (!tint || !/^#[0-9a-f]{6}$/i.test(tint)) return null;
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(tint.slice(i, i + 2), 16) / 255);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.6 ? '#3a2a10' : '#fff';
  });
  const words = $derived([value, state].filter((x) => x != null && x !== '').join(' · '));
  const said = $derived(words ? t('commun.action', { label, action: words }) : label);
</script>

{#snippet inside()}
  <span class="top">
    <span class="disc" style:background={on && tint ? tint : null} style:color={on && tintInk ? tintInk : null}>
      {#if glyph}{glyph}{:else}<Icon name={icon} size={compact ? 24 : 22} />{/if}
    </span>
    {#if value != null && !row}<b class="value num">{value}</b>{/if}
    {#if badge}<span class="badge"><Icon name={badge} size={13} /></span>{/if}
  </span>
  <span class="text">
    <b>{label}</b>
    {#if state}<small>{state}</small>{/if}
  </span>
  {#if value != null && row}<b class="value num">{value}</b>{/if}
{/snippet}

<div class="wrap" class:compact class:row>
  {#if href}
    <a class="tile" class:on class:dim class:busy class:open data-tone={tone} {href} title={compact ? label : null} aria-label={compact ? said : null}>
      {@render inside()}
    </a>
  {:else if fixed}
    <div class="tile fixed" class:on class:dim class:busy data-tone={tone} role="group" title={compact ? label : null} aria-label={said}>
      {@render inside()}
    </div>
  {:else}
    <button
      class="tile"
      class:on
      class:dim
      class:busy
      class:open
      data-tone={tone}
      {onclick}
      title={compact ? label : null}
      aria-label={said}
      aria-pressed={pressed == null ? undefined : pressed}
      aria-expanded={expanded == null ? undefined : expanded}>
      {@render inside()}
    </button>
  {/if}
  {#if onmore}
    <button class="more" class:open={moreOpen} onclick={onmore} aria-expanded={moreOpen} title={moreLabel} aria-label={moreLabel}>
      <Icon name={moreIcon} size={13} />
    </button>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    min-width: 0;
  }

  .tile {
    all: unset;
    box-sizing: border-box;
    display: grid;
    grid-template-rows: auto 1fr;
    gap: var(--sp-2);
    width: 100%;
    height: 100%;
    min-height: 104px;
    padding: var(--sp-3);
    border-radius: var(--r-md);
    background: var(--surface-2);
    color: var(--ink-2);
    cursor: pointer;
    text-align: left;
    transition:
      background 0.3s var(--ease),
      box-shadow 0.3s var(--ease),
      transform 0.15s var(--ease);
  }

  .tile.fixed {
    cursor: default;
  }

  .tile:not(.fixed):active {
    transform: scale(0.97);
  }

  .tile:focus-visible {
    outline: 3px solid var(--cool);
    outline-offset: -3px;
  }

  /* Lit: an opaque tile that comes forward, the disc in colour. */
  .tile.on {
    background: var(--tile-on);
    color: var(--ink);
    box-shadow: var(--shadow);
  }

  .tile.open {
    box-shadow: inset 0 0 0 2px var(--ink-3);
  }

  .tile.dim {
    opacity: 0.5;
  }

  [data-tone='warm'] {
    --tone: var(--warm);
    --tone-ink: var(--warm-ink);
  }

  [data-tone='cool'] {
    --tone: var(--cool);
    --tone-ink: var(--cool);
  }

  [data-tone='good'] {
    --tone: var(--good);
    --tone-ink: var(--good);
  }

  [data-tone='alert'] {
    --tone: var(--alert);
    --tone-ink: var(--alert);
  }

  .top {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .disc {
    flex: none;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-3);
    color: var(--ink-3);
    font-weight: 700;
    transition:
      background 0.3s var(--ease),
      box-shadow 0.3s var(--ease);
  }

  .on .disc {
    background: var(--tone);
    color: #fff;
    box-shadow: 0 0 0 5px color-mix(in srgb, var(--tone) 18%, transparent);
  }

  .busy .disc {
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.5;
    }
  }

  .value {
    margin-left: auto;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--ink);
    white-space: nowrap;
  }

  .badge {
    display: inline-grid;
    color: var(--ink-3);
  }

  .on .badge {
    color: var(--tone-ink);
  }

  .text {
    align-self: end;
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .text b {
    font-size: var(--fs-md);
    font-weight: 650;
    line-height: 1.2;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .text small {
    font-size: var(--fs-xs);
    font-weight: 550;
    color: var(--ink-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .on .text small {
    color: var(--tone-ink);
  }

  /* Side by side, low: a reading, a person. */
  .row .tile {
    grid-template-columns: auto minmax(0, 1fr) auto;
    grid-template-rows: auto;
    align-items: center;
    min-height: 60px;
    padding: var(--sp-2) var(--sp-3) var(--sp-2) var(--sp-2);
  }

  .row .disc {
    width: 34px;
    height: 34px;
  }

  .row .text {
    align-self: center;
  }

  .row .value {
    font-size: 18px;
  }

  /* The disc alone, square. */
  .compact .tile {
    min-height: 64px;
    padding: 0;
    grid-template-rows: auto;
    place-items: center;
  }

  .compact .text,
  .compact .value,
  .compact .badge {
    display: none;
  }

  .more {
    position: absolute;
    top: 8px;
    right: 8px;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 50%;
    background: var(--surface-3);
    color: var(--ink-2);
    cursor: pointer;
  }

  .compact .more {
    top: 2px;
    right: 2px;
    width: 20px;
    height: 20px;
  }

  .more.open {
    background: var(--tone, var(--warm));
    color: #fff;
  }
</style>
