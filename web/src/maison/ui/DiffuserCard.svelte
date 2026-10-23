<script>
  import { value, act, nameOf, isOn, reachable } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** An essential-oil diffuser (Tuya `mist_mode`): on/off, mist, timer,
   *  and its light: scene (gradient, fixed colour, night light), colour,
   *  intensity. The values are the device's own (`moli-tuya`, `complete`). */
  let { id, name = null } = $props();

  const label = $derived(nameOf(id, name));
  const on = $derived(isOn(id, 'switch'));
  const mist = $derived(value(id, 'mist_mode'));
  const timer = $derived(String(value(id, 'countdown') ?? '0'));
  const left = $derived(value(id, 'countdown_left'));
  const light = $derived(value(id, 'switch_led') === true);
  const scene = $derived(String(value(id, 'light_scene') ?? '1'));
  // `rrggbbhhhhssvv`: the colour as shown, its hue, saturation, and its
  // intensity (0–100 when the app dims it).
  const colour = $derived(/^[0-9a-f]{14}$/i.test(value(id, 'colour_data') ?? '') ? value(id, 'colour_data').toLowerCase() : null);
  const intensity = $derived.by(() => {
    if (scene === '2') return colour ? Math.min(100, parseInt(colour.slice(12), 16)) : 100;
    const b = value(id, 'bright_value');
    return typeof b === 'number' ? Math.round((b / 255) * 100) : 100;
  });

  // [value on the device, catalogue key]
  const MISTS = [
    ['off', 'commun.diffuseur.brume_arret'],
    ['small', 'commun.diffuseur.brume_faible'],
    ['big', 'commun.diffuseur.brume_forte'],
  ];
  const TIMERS = [
    ['0', 'commun.diffuseur.minuterie_sans'],
    ['1', 'commun.diffuseur.minuterie_1h'],
    ['2', 'commun.diffuseur.minuterie_3h'],
    ['3', 'commun.diffuseur.minuterie_6h'],
  ];
  const SCENES = [
    ['1', 'commun.diffuseur.scene_degrade'],
    ['2', 'commun.diffuseur.scene_couleur'],
    ['3', 'commun.diffuseur.scene_veilleuse'],
  ];
  // [catalogue key, hue in degrees]
  const HUES = [
    ['commun.lampe.couleur_rouge', 0],
    ['commun.lampe.couleur_orange', 30],
    ['commun.lampe.couleur_jaune', 55],
    ['commun.lampe.couleur_vert', 120],
    ['commun.lampe.couleur_cyan', 180],
    ['commun.lampe.couleur_bleu', 230],
    ['commun.lampe.couleur_violet', 275],
    ['commun.lampe.couleur_rose', 315],
  ];

  const hex2 = (n) => Math.round(n).toString(16).padStart(2, '0');
  /** Full-saturation hue at `pct` % → `rrggbbhhhhssvv`. */
  function encode(hue, pct) {
    const f = (n) => {
      const k = (n + hue / 60) % 6;
      return 1 - Math.max(0, Math.min(k, 4 - k, 1));
    };
    const scale = (255 * pct) / 100;
    return `${hex2(f(5) * scale)}${hex2(f(3) * scale)}${hex2(f(1) * scale)}${Math.round(hue).toString(16).padStart(4, '0')}ff${hex2(pct)}`;
  }
  const swatch = (hue) => `#${encode(hue, 100).slice(0, 6)}`;
  const hueOf = (c) => (c ? parseInt(c.slice(6, 10), 16) : null);
  const say = (key) => t('commun.action', { label, action: t(key).toLowerCase() });

  function power() {
    act(`${id}/switch`, !on, t(on ? 'commun.clim.label_arret' : 'commun.clim.label_marche', { name: label }));
  }
  function setMist(m, key) {
    if (m !== 'off' && !on) act(`${id}/switch`, true, t('commun.clim.label_marche', { name: label }));
    act(`${id}/mist_mode`, m, say(key));
  }
  function setScene(s, key) {
    if (!light) act(`${id}/switch_led`, true, say('commun.diffuseur.lumiere'));
    act(`${id}/light_scene`, s, say(key));
  }
  function paint(hue, key) {
    if (scene !== '2') act(`${id}/light_scene`, '2', say('commun.diffuseur.scene_couleur'));
    if (!light) act(`${id}/switch_led`, true, say('commun.diffuseur.lumiere'));
    act(`${id}/colour_data`, encode(hue, Math.max(intensity, 10)), say(key));
  }
  let dragging = $state(null);
  function commitIntensity(e) {
    const pct = Number(e.currentTarget.value);
    dragging = null;
    if (scene === '2') act(`${id}/colour_data`, encode(hueOf(colour) ?? 0, pct), t('commun.diffuseur.intensite', { label }));
    else act(`${id}/bright_value`, Math.round((pct / 100) * 255), t('commun.diffuseur.intensite', { label }));
  }
  const remaining = $derived.by(() => {
    if (typeof left !== 'number' || left <= 0) return null;
    const h = Math.floor(left / 60);
    const m = left % 60;
    return h ? t('commun.diffuseur.reste_h', { h, m: String(m).padStart(2, '0') }) : t('commun.diffuseur.reste_min', { m });
  });
</script>

<div class="diffuser" class:on class:off-grid={!reachable(id)}>
  <button class="head" onclick={power} aria-pressed={on} aria-label={t('commun.action', { label, action: t(on ? 'commun.piece.en_marche' : 'commun.piece.eteinte').toLowerCase() })}>
    <span class="ico"><Icon name="flower-outline" size={22} /></span>
    <span class="text">
      <b>{label}</b>
      <small>{on ? [t(MISTS.find(([m]) => m === mist)?.[1] ?? 'commun.piece.en_marche'), remaining].filter(Boolean).join(' · ') : t('commun.piece.eteinte')}</small>
    </span>
    <span class="switch" aria-hidden="true"><i></i></span>
  </button>

  <div class="row" role="group" aria-label={t('commun.diffuseur.brume')}>
    <Icon name="water" size={16} />
    {#each MISTS as [m, key] (m)}
      <button class="seg" class:sel={on && mist === m} onclick={() => setMist(m, key)}>{t(key)}</button>
    {/each}
  </div>

  {#if on}
    <div class="row" role="group" aria-label={t('commun.diffuseur.minuterie')}>
      <Icon name="timer" size={16} />
      {#each TIMERS as [v, key] (v)}
        <button class="seg" class:sel={timer === v} onclick={() => act(`${id}/countdown`, v, say(key))}>{t(key)}</button>
      {/each}
    </div>
  {/if}

  <div class="row" role="group" aria-label={t('commun.diffuseur.lumiere')}>
    <button class="seg light" class:sel={light} onclick={() => act(`${id}/switch_led`, !light, say('commun.diffuseur.lumiere'))} aria-pressed={light} title={t('commun.diffuseur.lumiere')}>
      <Icon name="light" size={16} />
    </button>
    {#each SCENES as [s, key] (s)}
      <button class="seg" class:sel={light && scene === s} onclick={() => setScene(s, key)}>{t(key)}</button>
    {/each}
  </div>

  {#if light}
    {#if scene !== '1'}
      <input
        class="level"
        type="range"
        min="0"
        max="100"
        value={dragging ?? intensity}
        oninput={(e) => (dragging = Number(e.currentTarget.value))}
        onchange={commitIntensity}
        aria-label={t('commun.diffuseur.intensite', { label })} />
    {/if}
    {#if scene === '2'}
      <div class="palette" role="group" aria-label={t('commun.lampe.couleurs', { label })}>
        {#each HUES as [key, hue] (hue)}
          <button class="dot" class:sel={hueOf(colour) != null && Math.abs(hueOf(colour) - hue) < 8} style:background={swatch(hue)} onclick={() => paint(hue, key)} title={t(key)} aria-label={t(key)}></button>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .diffuser {
    display: grid;
    gap: 10px;
    padding: 12px 14px;
    background: var(--surface-2);
    border-radius: var(--r-md);
    color: var(--ink-2);
  }

  .off-grid {
    opacity: 0.6;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    border: 0;
    background: none;
    padding: 0;
    text-align: left;
    color: inherit;
    cursor: pointer;
  }

  .ico {
    flex: none;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 12px;
    background: var(--surface);
  }

  .on .ico {
    background: var(--cool-soft);
    color: var(--cool);
  }

  .text {
    flex: 1;
    display: grid;
    min-width: 0;
  }

  .text b {
    color: var(--ink);
    font-weight: 650;
  }

  .text small {
    font-size: 12.5px;
  }

  .switch {
    flex: none;
    position: relative;
    width: 42px;
    height: 26px;
    border-radius: 999px;
    background: var(--surface-3);
    transition: background 0.25s var(--ease);
  }

  .switch i {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fff;
    box-shadow: var(--shadow);
    transition: transform 0.25s var(--ease);
  }

  .on .switch {
    background: var(--cool);
  }

  .on .switch i {
    transform: translateX(16px);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .seg {
    border: 0;
    background: var(--surface);
    border-radius: 999px;
    padding: 7px 11px;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--ink-2);
    box-shadow: var(--shadow);
    cursor: pointer;
  }

  .seg.light {
    display: grid;
    place-items: center;
    padding: 6px 8px;
  }

  .seg.sel {
    background: var(--ink);
    color: var(--surface);
  }

  .level {
    width: 100%;
    accent-color: var(--warm);
  }

  .palette {
    display: flex;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .dot {
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 50%;
    box-shadow: var(--shadow);
    cursor: pointer;
  }

  .dot.sel {
    outline: 2px solid var(--ink);
    outline-offset: 2px;
  }
</style>
