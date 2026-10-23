<script>
  import { hub, value, act, isOn, powerKey, protectedRoom, num, nameOf, deviceKind, lampsOf, doorOpen, leak, motion, lowBattery, unpowered, isFixture, note, reachable, quiet, outletsOf, speakerState, speakerGroup, heatedRoom, deg } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';
  import LightTile from './LightTile.svelte';
  import Tile from './Tile.svelte';
  import ClimateCard from './ClimateCard.svelte';
  import CoverTile from './CoverTile.svelte';
  import DiffuserCard from './DiffuserCard.svelte';
  import SpeakerCard from './SpeakerCard.svelte';
  import Remote from './Remote.svelte';
  import AmbiancePicker from './AmbiancePicker.svelte';
  import { phase, PHASES } from '../lib/printers.js';

  /** One room: its temperature and a switch for its lights in the head,
   *  then a tile per thing (`Tile`): lights and plugs (a tap switches, the
   *  small button opens a lamp's dimmer, colours, fan), TV and speakers (a
   *  tap opens the remote or the card: what plays, play/pause, volume,
   *  « Mets… »), clims (a tap opens the dial), and what it senses as low tiles.
   *  `icons`: tiles show the disc only (names on hover and for screen
   *  readers). `every`: each sensor shows, even silent (filtered view). */
  /** `onedit`: the room's sheet (rename, icon, other names), from Pièces. */
  let { room, every = false, icons = false, onedit = null } = $props();

  const locked = $derived(protectedRoom(room.name));

  function chips(d) {
    // An appliance that is no button: what it draws, nothing when unplugged.
    if (quiet(d.id)) {
      if (d.online === false) return every ? [{ tone: '', icon: 'alert', text: t('commun.piece.hors_ligne', { name: nameOf(d.id) }) }] : [];
      const w = value(d.id, 'cur_power') ?? value(d.id, 'power');
      if (typeof w === 'number' && w >= 3) return [{ tone: 'warm', icon: 'power', text: t('commun.piece.consomme', { name: nameOf(d.id), watts: num(w) }) }];
      return every ? [{ tone: '', icon: 'power', text: t('commun.piece.au_repos', { name: nameOf(d.id) }) }] : [];
    }
    // A silent sensor's last readings are old news: say it is silent.
    if (d.online === false && deviceKind(d) === 'sensor') {
      const dead = lowBattery(d.id);
      return [{ tone: dead ? 'alert' : '', icon: 'alert', text: t(dead ? 'commun.piece.pile_vide' : 'commun.piece.hors_ligne', { name: nameOf(d.id) }) }];
    }
    const out = [];
    const v = (k) => value(d.id, k);
    if (v('smoke') === true) out.push({ tone: 'alert', icon: 'fire', text: t('commun.capteur.fumee') });
    if (leak(d.id)) out.push({ tone: 'alert', icon: 'water', text: t('commun.piece.fuite', { name: nameOf(d.id) }) });
    const open = doorOpen(d.id);
    if (open === true) out.push({ tone: 'warm', icon: 'door-open', text: t('commun.piece.ouverte', { name: nameOf(d.id) }) });
    else if (open === false) out.push({ tone: '', icon: 'door-closed', text: t('commun.piece.fermee', { name: nameOf(d.id) }) });
    // The temperature is the room's, in the head.
    const hum = v('humidity') ?? v('humidity_value');
    if (typeof hum === 'number') out.push({ tone: '', icon: 'humidity', text: t('commun.pourcent', { n: num(hum) }) });
    const co2 = v('co2_value') ?? v('co2');
    if (typeof co2 === 'number') out.push({ tone: co2 > 1200 ? 'warm' : '', icon: 'leaf', text: `CO₂ ${num(co2)} ppm` });
    if (motion(d.id)) out.push({ tone: 'warm', icon: 'motion', text: t('commun.piece.presence') });
    if (lowBattery(d.id)) out.push({ tone: 'alert', icon: 'alert', text: t('commun.piece.pile_faible', { name: nameOf(d.id) }) });
    // A heated room (Netatmo): what it aims at, and its week of heating.
    if (d.points.some((p) => p.key === 'setpoint') && d.points.some((p) => p.key === 'setpoint_mode'))
      out.push({ tone: v('heating') === true ? 'warm' : '', icon: 'calendar', text: t('maison.chauffage.lien_piece', { name: nameOf(d.id), temp: num(v('setpoint'), 1) }), href: '#/chauffage' });
    if (deviceKind(d) === 'camera') out.push({ tone: 'cool', icon: 'cctv', text: nameOf(d.id), href: '#/cameras' });
    if (deviceKind(d) === 'printer') {
      const st = phase(v('state'));
      const text = st === 'printing' || st === 'paused' ? `${PHASES[st].label.toLowerCase()} ${t('commun.pourcent', { n: Math.round(Number(v('progress') ?? 0)) })}` : PHASES[st].label.toLowerCase();
      out.push({ tone: PHASES[st].tone, icon: 'printer3d', text: `${nameOf(d.id)} · ${d.online === false ? t('commun.piece.imprimante_eteinte') : text}`, href: '#/impression' });
    }
    if (every && !out.length) {
      // Its first reading, so that a quiet sensor still shows it is there.
      const p = d.points.find((p) => p.access?.read !== false && value(d.id, p.key) != null && value(d.id, p.key) !== '');
      const x = p ? value(d.id, p.key) : null;
      const shown = typeof x === 'number' ? num(x, 1) : x === true ? t('commun.oui').toLowerCase() : x === false ? t('commun.non').toLowerCase() : x;
      out.push({ tone: '', icon: 'info', text: p ? t('commun.piece.lecture', { name: nameOf(d.id), label: p.label, value: `${shown}${p.unit ? ` ${p.unit}` : ''}` }) : nameOf(d.id) });
    }
    return out;
  }

  const sensorChips = $derived(room.sensors.flatMap(chips));
  const lamps = $derived(lampsOf(room));
  let showLooks = $state(false);

  // ---- the room's temperature ----------------------------------------------------------

  const temp = $derived.by(() => {
    const readings = [...room.sensors, ...room.climates]
      .filter((d) => d.online !== false)
      .map((d) => value(d.id, 'temperature') ?? value(d.id, 'temp_current'))
      .filter((x) => typeof x === 'number');
    return readings.length ? readings.reduce((a, b) => a + b, 0) / readings.length : null;
  });
  // What the heating aims at here (a Netatmo room), beside the temperature.
  const setpoint = $derived.by(() => {
    const d = room.sensors.find((x) => heatedRoom(x) && x.online !== false);
    const sp = d ? value(d.id, 'setpoint') : null;
    return typeof sp === 'number' && !['off', 'hg'].includes(value(d.id, 'setpoint_mode')) ? sp : null;
  });
  // A room that could say its temperature but has nothing to read it with.
  const noThermometer = $derived(temp == null && !room.note && !room.empty);
  const tone = (deg) => (deg < 18 ? 'cold' : deg < 20 ? 'cool' : deg <= 24 ? 'good' : deg <= 27 ? 'warm' : 'hot');
  const gaugeAt = (deg) => Math.min(100, Math.max(0, ((deg - 14) / 16) * 100));

  // ---- the room's switch: its lights, never its TV or plugs (D12) -----------------------

  function roomPower() {
    const on = !room.lit;
    for (const d of room.lights) {
      if (isOn(d.id) === on || (on && unpowered(d.id) && !isFixture(d))) continue;
      const key = powerKey(d);
      if (key) act(`${d.id}/${key}`, typeof value(d.id, key) === 'string' ? (on ? 'ON' : 'OFF') : on, t(on ? 'commun.piece.label_allumer' : 'commun.piece.label_eteindre', { name: room.name }));
    }
  }

  // ---- pads ----------------------------------------------------------------------------

  const hasFan = (id) => hub.devices[id]?.points.some((p) => p.key === 'fan_switch');
  const fine = (d) => d.points.some((p) => ['brightness', 'color', 'color_temp'].includes(p.key) && p.access?.write);

  /** A speaking icon, from what it is and what it is called. */
  function padIcon(d) {
    const kind = deviceKind(d);
    const name = (d.label?.name || d.native_name || '').toLowerCase();
    if (kind === 'tv') return 'tv';
    if (kind === 'speaker') return speakerGroup(d) ? 'speaker-multiple' : 'speaker';
    if (kind === 'plug') return /vmc|ventil|\bfan\b|exhaust/.test(name) ? 'fan' : 'socket';
    if (hasFan(d.id) || (isFixture(d) && d.members?.some(hasFan))) return 'ceiling-fan-light';
    if (/spot/.test(name)) return 'spotlight';
    if (/meuble tv|télé|tele|tv unit|tv stand|television/.test(name)) return 'tv-light';
    if (/bureau|desk/.test(name)) return 'desk-lamp';
    if (/applique|sconce/.test(name)) return 'sconce';
    if (/ruban|bandeau|led|strip/.test(name)) return 'led-strip';
    if (/lampe|globe|fraise|lamp/.test(name)) return 'floor-lamp';
    if (/suspens|plafon|principal|cuisine|couloir|véranda|veranda|studio|combles|chambre|entrée|toilette|ceiling|pendant|\bmain\b|kitchen|hallway|bedroom|attic|bathroom|toilet/.test(name)) return 'ceiling-light';
    return 'light';
  }

  /** A strip's pad for all its outlets (`ALL`), then one per outlet. */
  const ALL = '*';
  const plugPads = (d) => {
    const outlets = outletsOf(d.id);
    if (!outlets.length) return [{ d, point: null }];
    return [{ d, point: ALL, outlets }, ...outlets.map((o) => ({ d, point: o.key, name: o.name }))];
  };
  const litCount = (outlets, id) => outlets.filter((o) => isOn(id, o.key)).length;

  /** The tiles: lights, then TV and speakers, then plugs. */
  const pads = $derived([
    ...room.lights.map((d) => ({ d, point: null })),
    ...room.media.map((d) => ({ d, point: deviceKind(d) === 'tv' ? 'power' : null, speaker: deviceKind(d) === 'speaker', tv: deviceKind(d) === 'tv' })),
    ...room.plugs.flatMap(plugPads),
  ]);

  let open = $state(null);

  function stateOf(d, point, outlets) {
    if (unpowered(d.id) && !isFixture(d)) return t('commun.piece.coupee');
    if (!reachable(d.id)) return t('commun.piece.injoignable');
    if (outlets) return t('commun.piece.prises_allumees', { on: litCount(outlets, d.id), count: outlets.length });
    if (deviceKind(d) === 'speaker') return t(`commun.enceinte.${speakerState(d.id)}`);
    if (!isOn(d.id, point)) return t('commun.piece.eteinte');
    const b = value(d.id, 'brightness');
    return typeof b === 'number' && fine(d) ? t('commun.pourcent', { n: Math.round(b) }) : t('commun.piece.allumee');
  }

  function tap(d, point, outlets, name) {
    if (unpowered(d.id) && !isFixture(d)) {
      note(t('commun.lampe.coupee_note', { label: nameOf(d.id) }));
      return;
    }
    if (outlets) {
      // All on → all off; otherwise everything on.
      const on = litCount(outlets, d.id) < outlets.length;
      for (const o of outlets) if (isOn(d.id, o.key) !== on) act(`${d.id}/${o.key}`, on, `${nameOf(d.id)} · ${o.name}`);
      return;
    }
    if (name) {
      act(`${d.id}/${point}`, !isOn(d.id, point), `${nameOf(d.id)} · ${name}`);
      return;
    }
    const key = point ?? powerKey(d);
    if (!key) return;
    const cur = value(d.id, key);
    act(`${d.id}/${key}`, typeof cur === 'string' ? (isOn(d.id, point) ? 'OFF' : 'ON') : !isOn(d.id, point), nameOf(d.id));
  }

  // The unit's modes (as it says them) → the catalogue key of their short name.
  const MODES = {
    froid: 'commun.piece.mode.froid',
    chaud: 'commun.piece.mode.chaud',
    auto: 'commun.piece.mode.auto',
    déshumidification: 'commun.piece.mode.deshumidification',
    ventilation: 'commun.piece.mode.ventilation',
  };
  let openClim = $state(null);
</script>

<section class="card room">
  <div class="head">
    <span class="room-icon" class:lit={room.lit}><Icon name={room.icon} size={22} /></span>
    <div class="title">
      <h2>{room.name}{#if locked}<span class="lock" title={t('commun.piece.protegee_titre')}><Icon name="lock" size={13} /></span>{/if}</h2>
      <span class="sub">
        <span class="muted">{room.note ?? (room.lit ? t('commun.piece.allumees', { count: room.lit }) : t('commun.piece.tout_eteint'))}{noThermometer ? ` · ${t('commun.piece.sans_thermometre')}` : ''}</span>
        {#if temp != null}
          <span class="temp {tone(temp)}" title={setpoint != null ? t('commun.piece.temperature_consigne', { temp: num(temp, 1), aim: deg(setpoint) }) : t('commun.piece.temperature')}>
            <b class="num">{num(temp, 1)}°</b>
            {#if setpoint != null}<span class="aim num" aria-label={t('commun.piece.consigne', { aim: deg(setpoint) })}>→ {deg(setpoint)}°</span>{:else}<i class="gauge"><i style="left:{gaugeAt(temp)}%"></i></i>{/if}
          </span>
        {/if}
      </span>
    </div>
    {#if lamps.length}
      <button class="round" class:open={showLooks} onclick={() => (showLooks = !showLooks)} aria-expanded={showLooks} title={t('commun.piece.ambiances')} aria-label={t('commun.piece.ambiances_de', { name: room.name })}>
        <Icon name="palette" size={17} />
      </button>
    {/if}
    {#if room.lights.length}
      <button class="room-switch" class:on={room.lit} onclick={roomPower} aria-pressed={!!room.lit} title={room.lit ? t('commun.piece.tout_eteindre') : t('commun.piece.tout_allumer')} aria-label={t(room.lit ? 'commun.piece.label_eteindre' : 'commun.piece.label_allumer', { name: room.name })}><i></i></button>
    {/if}
    {#if onedit}
      <button class="round" onclick={() => onedit(room)} title={t('commun.piece.modifier')} aria-label={t('commun.piece.modifier_nom', { name: room.name })}>
        <Icon name="tune" size={16} />
      </button>
    {/if}
  </div>

  {#if locked}
    <p class="guarded"><Icon name="lock" size={14} />{t('commun.piece.protegee_phrase')}</p>
  {/if}

  {#if room.empty}
    <p class="empty">
      <span>{t('commun.piece.vide')}</span>
      <a href="#/atelier">{t('commun.piece.vide_invitation')}</a>
    </p>
  {/if}

  {#if showLooks}
    <AmbiancePicker lights={lamps} label={room.name} />
  {/if}

  {#if pads.length}
    <div class="pads" class:icons>
      {#each pads as { d, point, speaker, tv, outlets, name } (`${d.id}/${point}`)}
        {@const lit = speaker ? speakerState(d.id) === 'playing' : outlets ? litCount(outlets, d.id) > 0 : isOn(d.id, point)}
        {@const cut = unpowered(d.id) && !isFixture(d)}
        {@const more = !speaker && !point && !cut && (fine(d) || hasFan(d.id) || (isFixture(d) && d.members?.some(hasFan)))}
        {@const label = name ?? nameOf(d.id)}
        {@const opens = speaker || tv || more}
        <Tile
          icon={outlets ? 'power' : padIcon(d)}
          {label}
          state={stateOf(d, point, outlets)}
          on={lit}
          tone={speaker ? 'good' : 'warm'}
          dim={cut || (!reachable(d.id) && !cut)}
          compact={icons}
          open={open === d.id}
          pressed={speaker ? null : lit}
          expanded={speaker ? open === d.id : null}
          onclick={() => (speaker ? (open = open === d.id ? null : d.id) : tap(d, point, outlets, name))}
          onmore={opens && !speaker ? () => (open = open === d.id ? null : d.id) : null}
          moreOpen={open === d.id}
          moreIcon={tv ? 'remote' : 'tune'}
          moreLabel={tv ? t('commun.piece.telecommande_nom', { name: nameOf(d.id) }) : t('commun.piece.regler_nom', { name: nameOf(d.id) })} />
      {/each}
    </div>
    {#if open && pads.some((p) => p.d.id === open && p.speaker)}
      <SpeakerCard id={open} inline />
    {:else if open && pads.some((p) => p.d.id === open && p.tv)}
      <div class="tv-remote"><Remote tv={open} keyboard={false} /></div>
    {:else if open && pads.some((p) => p.d.id === open)}
      <LightTile id={open} dimmer />
    {/if}
  {/if}

  {#each room.covers as d (d.id)}
    <CoverTile id={d.id} />
  {/each}

  {#each room.diffusers as d (d.id)}
    <DiffuserCard id={d.id} />
  {/each}

  {#if room.climates.length}
    <div class="pads" class:icons>
      {#each room.climates as d (d.id)}
        {@const on = isOn(d.id)}
        {@const mode = on ? (MODES[value(d.id, 'mode')] ? t(MODES[value(d.id, 'mode')]) : t('commun.piece.en_marche')) : t('commun.piece.eteinte')}
        <Tile
          icon="air-conditioner"
          label={nameOf(d.id)}
          state={`${num(value(d.id, 'temperature'), 0)}° → ${num(value(d.id, 'target_temperature'), 1)}° · ${mode}`}
          {on}
          tone="cool"
          compact={icons}
          open={openClim === d.id}
          expanded={openClim === d.id}
          onclick={() => (openClim = openClim === d.id ? null : d.id)} />
      {/each}
    </div>
    {#each room.climates as d (d.id)}
      {#if openClim === d.id}
        <ClimateCard id={d.id} name={nameOf(d.id)} />
      {/if}
    {/each}
  {/if}

  {#if sensorChips.length}
    <div class="readings">
      {#each sensorChips as c, i (i)}
        <Tile row static={!c.href} href={c.href ?? null} icon={c.icon} label={c.text} on={!!c.tone} tone={c.tone || 'cool'} />
      {/each}
    </div>
  {/if}
</section>

<style>
  /* A TV's remote, opened in its room's card. */
  .tv-remote {
    padding-top: 4px;
  }

  .room {
    display: grid;
    gap: var(--sp-3);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .empty {
    display: grid;
    gap: var(--sp-1);
    margin: 0;
    padding: 14px 16px;
    border-radius: var(--r-sm);
    background: var(--surface-2);
    color: var(--ink-3);
    font-size: var(--fs-md);
  }

  .empty a {
    color: var(--cool);
    font-weight: 600;
  }

  .room-icon {
    flex: none;
    width: 40px;
    height: 40px;
    border-radius: 14px;
    display: grid;
    place-items: center;
    background: var(--surface-2);
    color: var(--ink-3);
  }

  .room-icon.lit {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  .title {
    flex: 1;
    display: grid;
    min-width: 0;
  }

  .title h2 {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 17px;
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .lock {
    display: inline-grid;
    color: var(--cool);
  }

  .title .sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
  }

  .sub > .muted {
    font-size: 12.5px;
  }

  /* The room's temperature, with a cold-to-hot gauge. */
  .temp {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 9px;
    border-radius: 999px;
    background: var(--surface-2);
    font-size: var(--fs-sm);
  }

  .temp b {
    font-weight: 700;
  }

  .temp .aim {
    font-weight: 550;
    color: var(--ink-3);
  }

  /* A protected room, said in a sentence (the lock is in the title). */
  .guarded {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    padding: 8px 12px;
    border-radius: 12px;
    background: var(--cool-soft);
    color: var(--cool);
    font-size: var(--fs-sm);
  }

  .gauge {
    position: relative;
    width: 28px;
    height: 5px;
    border-radius: 3px;
    background: linear-gradient(90deg, #4f8cff 0%, #3fc1d9 25%, #4cc26a 40%, #4cc26a 62%, #ffb340 80%, #ff5a3c 100%);
  }

  .gauge i {
    position: absolute;
    top: -3px;
    width: 3px;
    height: 11px;
    border-radius: 2px;
    background: var(--ink);
    box-shadow: 0 0 0 1.5px var(--surface-2);
    transform: translateX(-50%);
  }

  .cold b {
    color: #3b7bea;
  }

  .cool b {
    color: #2aa5bd;
  }

  .good b {
    color: #2f9e55;
  }

  .warm b {
    color: #e08a00;
  }

  .hot b {
    color: #e5482b;
  }

  .round {
    flex: none;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border: none;
    border-radius: 50%;
    background: var(--surface-2);
    color: var(--ink-2);
    cursor: pointer;
  }

  .round.open {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  /* The room's lights, all at once: a small rocker. */
  .room-switch {
    flex: none;
    position: relative;
    width: 42px;
    height: 26px;
    border: none;
    border-radius: 999px;
    background: var(--surface-3);
    cursor: pointer;
    transition: background 0.25s var(--ease);
  }

  .room-switch i {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--surface);
    box-shadow: 0 1px 3px rgb(0 0 0 / 20%);
    transition: transform 0.25s var(--ease);
  }

  .room-switch.on {
    background: var(--warm);
  }

  .room-switch.on i {
    transform: translateX(16px);
  }

  /* Tiles, two or more side by side; the discs alone when asked. */
  .pads {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
    gap: var(--sp-2);
  }

  .pads.icons {
    grid-template-columns: repeat(auto-fill, minmax(64px, 1fr));
  }

  /* What the room senses: low tiles, two or more side by side. */
  .readings {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--sp-2);
  }
</style>
