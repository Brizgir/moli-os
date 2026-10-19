<script>
  import { locale } from '../../lib/i18n.svelte.js';
  import { timeTicks, linePath, stepPath, valueAt } from '../lib/heating-curves.js';

  /** One heating chart, from `from` to `to` (ms) on the temperature `axis`
   *  (`scale()`): temperature `lines` (`[{ id, points, color?, faint? }]`),
   *  the setpoint's `steps`, the `heat` band under them. A finger or the
   *  mouse picks a moment: `onhover(ts)`, shown at `hover` (shared by the
   *  charts of the page). */
  let { from, to, axis, range = 'day', lines = [], steps = [], heat = null, hover = null, onhover = () => {}, label = '' } = $props();

  let W = $state(320);
  const H = 172;
  const T = 14;
  const BOTTOM = H - 34;
  const BAND = H - 29;

  const x = (ts) => ((ts - from) / Math.max(1, to - from)) * W;
  const y = (v) => T + (BOTTOM - T) * (1 - (v - axis.lo) / Math.max(1e-9, axis.hi - axis.lo));

  const ticks = $derived(timeTicks(from, to, Math.max(2, Math.floor(W / 64))));
  function tick(t) {
    const d = new Date(t.ts);
    if (range === 'day' && !t.day) return d.toLocaleTimeString(locale(), { hour: '2-digit' });
    if (range === 'month') return d.toLocaleDateString(locale(), { day: 'numeric', month: 'short' }).replace('.', '');
    return d.toLocaleDateString(locale(), { weekday: 'short' }).replace('.', '');
  }
  const anchor = (px) => (px < 18 ? 'start' : px > W - 18 ? 'end' : 'middle');

  function pick(e) {
    const box = e.currentTarget.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - box.left) / Math.max(1, box.width)));
    onhover(from + f * (to - from));
  }
</script>

<div class="plot" bind:clientWidth={W}>
  <svg
    viewBox="0 0 {W} {H}"
    height={H}
    role="img"
    aria-label={label}
    onpointerdown={pick}
    onpointermove={(e) => (e.pointerType === 'mouse' || e.buttons) && pick(e)}
    onpointerleave={(e) => e.pointerType === 'mouse' && onhover(null)}
  >
    {#each axis.ticks as v (v)}
      <line x1="0" x2={W} y1={y(v)} y2={y(v)} class="rule" />
    {/each}
    {#each ticks as t (t.ts)}
      <line x1={x(t.ts)} x2={x(t.ts)} y1={T} y2={BOTTOM} class="rule" class:strong={t.day && range === 'day'} />
    {/each}

    {#if heat}
      <rect x="0" y={BAND} width={W} height="6" rx="3" class="band-bg" />
      {#each heat as h (h.from)}
        <rect x={x(h.from)} y={BAND} width={Math.max(1.5, x(h.to) - x(h.from))} height="6" rx="2" class="band" opacity={0.35 + 0.65 * h.share} />
      {/each}
    {/if}

    {#if steps.length}
      <path d={stepPath(steps, x, y)} class="setpoint" />
    {/if}
    {#each lines as l (l.id)}
      <path d={linePath(l.points, x, y)} class="temp" class:faint={l.faint} style:stroke={l.color} />
    {/each}

    {#if hover != null}
      <line x1={x(hover)} x2={x(hover)} y1={T} y2={heat ? BAND + 6 : BOTTOM} class="cursor" />
      {#each lines as l (l.id)}
        {@const v = valueAt(l.points, hover)}
        {#if v != null}<circle cx={x(hover)} cy={y(v)} r="3.5" class="dot" style:fill={l.color} />{/if}
      {/each}
    {/if}

    {#each axis.ticks as v, i (v)}
      {#if i % Math.ceil(axis.ticks.length / 4) === 0 || i === axis.ticks.length - 1}
        <text x="2" y={y(v) - 3} class="axis halo">{v}°</text>
      {/if}
    {/each}
    {#each ticks as t (t.ts)}
      <text x={x(t.ts)} y={H - 6} class="axis" text-anchor={anchor(x(t.ts))}>{tick(t)}</text>
    {/each}
  </svg>
</div>

<style>
  .plot {
    width: 100%;
    min-width: 0;
    overflow: hidden;
  }

  /* The width follows the card (never the other way round). */
  svg {
    display: block;
    width: 100%;
    touch-action: pan-y;
    cursor: crosshair;
  }

  .rule {
    stroke: var(--line);
    stroke-dasharray: 3 5;
  }

  .rule.strong {
    stroke-dasharray: none;
  }

  .axis {
    font-size: 11px;
    fill: var(--ink-3);
    font-weight: 600;
  }

  .halo {
    paint-order: stroke;
    stroke: var(--surface);
    stroke-width: 4px;
    stroke-linejoin: round;
  }

  .band-bg {
    fill: var(--surface-3);
  }

  .band {
    fill: var(--warm);
  }

  .setpoint {
    fill: none;
    stroke: var(--warm);
    stroke-width: 1.5;
    stroke-linejoin: round;
  }

  .temp {
    fill: none;
    stroke: var(--cool);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .temp.faint {
    stroke-width: 1.2;
    stroke-dasharray: 2 3;
  }

  .cursor {
    stroke: var(--ink-3);
    stroke-width: 1;
  }

  .dot {
    fill: var(--cool);
    stroke: var(--surface);
    stroke-width: 1.5;
  }
</style>
