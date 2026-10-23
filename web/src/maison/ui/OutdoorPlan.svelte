<script>
  // The outside of the house as the plan draws it: the ground floor from
  // above, its garden, terrace, drive and the house in the middle, each
  // outdoor space with its area, the garden with the weather's temperature,
  // the cameras and the doorbell where they stand. A tap opens the plan.
  import { home, hub, value, num } from '../lib/home.svelte.js';
  import { groundFloor, outdoorSpaces, miniPlan } from '../lib/outside.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  const floor = $derived(groundFloor(home.plan));
  const spaces = $derived(outdoorSpaces(floor));
  const pic = $derived(miniPlan(floor));
  const kinds = $derived(new Set(spaces.map((s) => s.kind)));
  const title = $derived(kinds.has('garden') && (kinds.has('terrace') || kinds.has('deck')) ? t('maison.dehors.jardin_terrasse') : t('maison.dehors.autour'));

  const outside = $derived.by(() => {
    const v = value(home.config?.outdoor?.weather, 'temperature');
    return typeof v === 'number' ? v : null;
  });
  // The first garden says the weather's temperature, as the plan does.
  const gardenId = $derived(pic?.rooms.find((r) => r.kind === 'garden')?.id ?? null);

  // The cameras placed on this floor: the doorbell in coral, the rest in blue.
  const cams = $derived(
    (pic?.devices ?? [])
      .map((d) => ({ ...d, device: hub.devices[d.id] }))
      .filter((d) => d.device?.camera || d.device?.points?.some((p) => p.key === 'doorbell'))
      .map((d) => ({ ...d, bell: d.device.points.some((p) => p.key === 'doorbell') })),
  );

  // Letters sized for the screen, whatever the plot's size in cm.
  let pxW = $state(0);
  let pxH = $state(0);
  const scale = $derived(pic ? Math.max((pic.w + 2 * pic.pad) / Math.max(1, pxW), (pic.h + 2 * pic.pad) / Math.max(1, pxH)) : 1);
  const px = (n) => n * scale;
  const points = (poly) => poly.map(([x, y]) => `${x},${y}`).join(' ');
</script>

{#if pic && spaces.length}
  <a class="card plan" href="#/plan">
    <div class="card-head">
      <h2><Icon name="flower-outline" size={18} />{title}</h2>
      <span class="more" aria-hidden="true"><Icon name="arrow-top-right" size={18} /></span>
    </div>
    <div class="frame" bind:clientWidth={pxW} bind:clientHeight={pxH}>
      <svg viewBox={pic.viewBox} role="img" aria-label={t('maison.dehors.plan_alt')}>
        {#each pic.rooms as r (r.id)}
          <polygon points={points(r.poly)} class="room {r.kind}" class:outdoor={r.outdoor} stroke-width={px(1.5)} />
        {/each}
        {#each pic.rooms as r (r.id)}
          {#if r.outdoor && r.w > px(60) && r.h > px(30)}
            <text x={r.cx} y={r.cy - px(2)} text-anchor="middle" class="name" font-size={px(13)}>{r.name}</text>
            <text x={r.cx} y={r.cy + px(14)} text-anchor="middle" class="fact num" font-size={px(12)}>
              {#if r.id === gardenId && outside != null}{num(outside, 0)}°{' · '}{/if}{t('maison.dehors.m2', { n: num(spaces.find((s) => s.id === r.id)?.area ?? 0, 0) })}
            </text>
          {/if}
        {/each}
        {#if pic.house && pic.house.w > px(70) && pic.house.h > px(20)}
          <text x={pic.house.cx} y={pic.house.cy + px(4)} text-anchor="middle" class="fact" font-size={px(12)}>{t('maison.dehors.la_maison')}</text>
        {/if}
        {#each cams as c (c.id)}
          <circle cx={c.x} cy={c.y} r={px(6)} class="cam" class:bell={c.bell} stroke-width={px(2)} />
        {/each}
      </svg>
    </div>
    <div class="spaces">
      {#each spaces as s (s.id)}
        <span class="space"><i class={s.kind}></i>{s.name} <small class="num">{t('maison.dehors.m2', { n: num(s.area, 0) })}</small></span>
      {/each}
      {#if cams.some((c) => !c.bell)}<span class="space"><i class="dot cam"></i>{t('maison.dehors.legende_camera')}</span>{/if}
      {#if cams.some((c) => c.bell)}<span class="space"><i class="dot bell"></i>{t('maison.dehors.legende_sonnette')}</span>{/if}
    </div>
  </a>
{/if}

<style>
  .plan {
    --garden: #d9e6c8;
    --deck: #cfe0b8;
    --terrace: #e7d9c0;
    --pool: #bcdcf0;
    --driveway: #dcdfe4;
    --house: var(--surface-2);
    --house-line: #c9cfd9;

    display: block;
    color: inherit;
    text-decoration: none;
    transition: transform 0.2s var(--ease);
  }

  :global(.maison[data-theme='night']) .plan {
    --garden: #1b2a1c;
    --deck: #1f3121;
    --terrace: #2a2419;
    --pool: #16283a;
    --driveway: #1e222b;
    --house: var(--surface-3);
    --house-line: #3a4150;
  }

  .plan:hover {
    transform: translateY(-2px);
  }

  .frame {
    border-radius: var(--r-md);
    background: var(--surface-2);
    overflow: hidden;
    display: grid;
    place-items: center;
    min-height: 160px;
    max-height: 340px;
  }

  svg {
    width: 100%;
    height: 100%;
    max-height: 340px;
    display: block;
  }

  .room {
    fill: var(--house);
    stroke: var(--house-line);
  }

  .room.garden {
    fill: var(--garden);
    stroke: none;
  }

  .room.deck {
    fill: var(--deck);
    stroke: none;
  }

  .room.terrace {
    fill: var(--terrace);
    stroke: none;
  }

  .room.pool {
    fill: var(--pool);
    stroke: none;
  }

  .room.driveway {
    fill: var(--driveway);
    stroke: none;
  }

  .name {
    font-weight: 700;
    fill: var(--ink);
  }

  .fact {
    fill: var(--ink-3);
    font-weight: 600;
  }

  .cam {
    fill: var(--cool);
    stroke: var(--surface);
  }

  .cam.bell {
    fill: var(--alert);
  }

  .spaces {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    margin-top: 14px;
  }

  .space {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 7px 12px;
    border-radius: 999px;
    background: var(--surface-2);
    font-size: var(--fs-md);
    font-weight: 600;
  }

  .space small {
    font-size: var(--fs-sm);
    color: var(--ink-3);
    font-weight: 600;
  }

  .space i {
    width: 12px;
    height: 12px;
    border-radius: 4px;
    display: inline-block;
    background: var(--house);
  }

  .space i.garden {
    background: var(--garden);
  }

  .space i.deck {
    background: var(--deck);
  }

  .space i.terrace {
    background: var(--terrace);
  }

  .space i.pool {
    background: var(--pool);
  }

  .space i.driveway {
    background: var(--driveway);
  }

  .space i.dot {
    border-radius: 50%;
  }

  .space i.cam {
    background: var(--cool);
  }

  .space i.bell {
    background: var(--alert);
  }
</style>
