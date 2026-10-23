<script>
  import { untrack } from 'svelte';
  import { hub, home, device, value, since, nameOf, roomOf, hidden, doorOpen, isOn, clock, relative } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';
  import Tile from './Tile.svelte';

  /** The house's safety at a glance: the doors, what moves, the alarm. A
   *  sensor gone silent says so: a dead battery, an unknown state or a
   *  missing device must never pass for « closed » or « calm ». */

  const CONTACT = ['contact', 'doorcontact_state'];
  const MOVES = ['occupancy', 'pir', 'motion'];
  const DAY = 86_400_000;
  /** A sensor that never says whether it is online (no availability) and
   *  has said nothing for this long is not trusted any more. */
  const STALE = 26 * 3_600_000;
  const has = (d, keys) => keys.find((k) => d?.points.some((p) => p.key === k)) ?? null;
  const battery = (id) => {
    const pct = value(id, 'battery_percentage') ?? value(id, 'battery');
    return typeof pct === 'number' ? Math.round(pct) : null;
  };
  /** « depuis 10:42 » today, « il y a 3 j » before. */
  const when = (ts) => (!ts ? '' : home.now - ts < DAY && new Date(ts).getDate() === new Date(home.now).getDate() ? t('salon.securite.depuis', { heure: clock(ts) }) : relative(ts, home.now));

  /** Why a sensor cannot be trusted now, or null. */
  function silence(dev, key) {
    if (!dev) return t('salon.securite.introuvable');
    if (dev.online === false) return t('salon.securite.hors_ligne');
    if (!key || value(dev.id, key) == null) return t('salon.securite.rien_recu');
    if (battery(dev.id) === 0) return t('salon.securite.pile_vide');
    const at = since(dev.id, key);
    if (dev.online == null && at && home.now - at > STALE) return t('salon.securite.rien_depuis', { depuis: relative(at, home.now) });
    return null;
  }

  const doors = $derived.by(() => {
    const listed = home.config?.doors ?? [];
    const known = new Set(listed.map((d) => d.id));
    const others = Object.values(hub.devices)
      .filter((d) => !known.has(d.id) && !hidden(d.id) && has(d, CONTACT))
      .map((d) => ({ id: d.id, name: nameOf(d.id) }));
    return [...listed, ...others].map((d) => {
      const dev = device(d.id);
      const key = has(dev, CONTACT);
      const open = doorOpen(d.id);
      // Unknown is not closed: not a boolean, it is silent.
      const why = silence(dev, key) ?? (typeof open === 'boolean' ? null : t('salon.securite.etat_inconnu'));
      return { ...d, key, open: why ? null : open, why, silent: !!why, at: key ? since(d.id, key) : null, pct: battery(d.id), seen: `${value(d.id, key)}` };
    });
  });

  // When each door last changed, from the history (it keeps changes only):
  // a sensor that repeats its state must not move « depuis 10:42 ».
  let changed = $state({});
  async function lastChange(id, key) {
    try {
      const res = await fetch(`/api/history?point=${encodeURIComponent(`${id}/${key}`)}&hours=720&points=5000`);
      if (!res.ok) return;
      const h = await res.json();
      // The last real transition: Moli records the value again at each of
      // its starts, which is not a door moving.
      let prev = h.before ? h.before[1] : undefined;
      let ts = null;
      for (const [at, v] of h.raw ?? []) {
        if (prev === undefined || v !== prev) ts = at;
        prev = v;
      }
      if (ts) changed[id] = ts;
    } catch {
      /* the state's own time stays */
    }
  }
  $effect(() => {
    for (const d of doors) {
      void d.seen;
      if (d.key) untrack(() => lastChange(d.id, d.key));
    }
  });

  // The cameras' names as the family knows them (« Devant la porte »).
  const camName = (id) => (home.config?.favorites?.cameras ?? []).find((c) => c.id === id)?.name;

  const sensors = $derived(
    Object.values(hub.devices)
      .filter((d) => !hidden(d.id) && !d.id.startsWith('presence:') && !d.id.startsWith('telephones:') && has(d, MOVES))
      .map((d) => {
        const key = has(d, MOVES);
        const v = value(d.id, key);
        const moving = v === true || v === 'pir' || Number(value(d.id, 'person')) > 0 || value(d.id, 'person') === true;
        const why = silence(d, key);
        return {
          id: d.id,
          name: camName(d.id) ?? nameOf(d.id),
          room: roomOf(d),
          outside: !!d.camera,
          moving: !why && moving,
          silent: !!why,
          why,
          at: since(d.id, key),
        };
      })
      .sort((a, b) => Number(a.outside) - Number(b.outside) || Number(b.moving) - Number(a.moving)),
  );

  // The alarm's switches (the « Sécurité » room of the layout); offline: unknown.
  const alarm = $derived(
    Object.values(hub.devices)
      .filter((d) => !hidden(d.id) && roomOf(d) === 'Sécurité')
      .map((d) => ({ id: d.id, name: nameOf(d.id), on: d.online === false ? null : isOn(d.id), ringing: /en cours/i.test(nameOf(d.id)) })),
  );

  const summary = $derived.by(() => {
    const open = doors.filter((d) => d.open === true);
    const unknown = doors.filter((d) => d.silent);
    const ringing = alarm.find((a) => a.on === true && a.ringing);
    const inside = sensors.filter((s) => s.moving && !s.outside);
    const muteSensors = sensors.filter((s) => s.silent).length;
    if (ringing) return { tone: 'alert', icon: 'alert', text: t('salon.securite.alarme', { nom: ringing.name }) };
    if (open.length) return { tone: 'warm', icon: 'door-open', text: t('salon.securite.ouvertes', { count: open.length, nom: open[0].name }) };
    if (unknown.length)
      return { tone: 'warm', icon: 'warning', text: t('salon.securite.inconnues', { count: unknown.length, nom: unknown[0].name }) };
    if (inside.length) return { tone: 'warm', icon: 'motion', text: t('salon.securite.mouvement', { lieux: inside.map((s) => s.room ?? s.name).join(', ') }) };
    if (!doors.length) return { tone: '', icon: 'shield', text: t('salon.securite.aucune_porte') };
    return { tone: 'good', icon: 'shield', text: muteSensors ? t('salon.securite.tout_ferme_muets', { count: muteSensors }) : t('salon.securite.tout_ferme') };
  });
</script>

<section class="card security">
  <div class="card-head">
    <h2><Icon name="shield" size={18} />{t('salon.securite.titre')}</h2>
    <span class="chip {summary.tone}"><Icon name={summary.icon} size={15} />{summary.text}</span>
  </div>

  {#if doors.length}
    <div class="tiles">
      {#each doors as d (d.id)}
        <Tile
          row
          static
          icon={d.silent ? 'offline' : d.open ? 'door-open' : 'door-closed'}
          label={d.name}
          state={d.silent ? t('salon.securite.muet', { raison: d.why }) : `${d.open ? t('salon.securite.ouverte') : t('salon.securite.fermee')}${(changed[d.id] ?? d.at) ? ` ${when(changed[d.id] ?? d.at)}` : ''}`}
          value={d.pct != null && !d.silent ? `${d.pct} %` : null}
          on={!d.silent}
          tone={d.open ? 'warm' : 'good'}
          dim={d.silent} />
      {/each}
    </div>
  {/if}

  {#if sensors.length}
    <div class="tiles">
      {#each sensors as s (s.id)}
        <Tile
          row
          static
          icon={s.outside ? 'cctv' : 'motion'}
          label={s.room && !s.outside && s.room !== s.name ? `${s.name} · ${s.room}` : s.name}
          state={s.silent ? t('salon.securite.detecteur_muet', { raison: s.why }) : `${s.moving ? t('salon.securite.mouvement_etat') : t('salon.securite.calme')}${s.at ? ` ${when(s.at)}` : ''}`}
          on={s.moving}
          tone="warm"
          dim={s.silent} />
      {/each}
    </div>
  {/if}

  {#if alarm.length}
    <div class="tiles">
      {#each alarm as a (a.id)}
        <Tile row static icon={a.ringing ? 'alert' : 'lock'} label={a.name} state={a.on == null ? '?' : a.on ? t('salon.oui') : t('salon.non')} on={a.on === true} tone={a.ringing ? 'alert' : 'good'} dim={a.on == null} />
      {/each}
    </div>
  {/if}
</section>

<style>
  .security {
    display: grid;
    gap: 14px;
  }

  .card-head {
    flex-wrap: wrap;
    gap: 10px;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: var(--sp-2);
  }
</style>
