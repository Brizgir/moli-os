<script>
  import { hub, value, act, pending } from '../lib/home.svelte.js';
  import { t, locale } from '../../lib/i18n.svelte.js';
  import Tile from '../ui/Tile.svelte';

  // The house's mode (a helper, « Mode Maison »): one tap, like Home
  // Assistant's mode buttons. Beside it, the speakers that talk
  // (announcements): one tap silences them all, for four hours at most.
  const modeDevice = $derived(Object.values(hub.devices).find((d) => d.id.endsWith(':mode_maison')));
  const modes = $derived(modeDevice?.points.find((p) => p.key === 'state')?.kind.values ?? []);
  const mode = $derived(modeDevice ? value(modeDevice.id, 'state') : null);
  const MODE_ICON = { Normal: 'home', Nuit: 'moon', Absent: 'lock', Vacances: 'palm' };
  /** A mode's name in the house's language (an unknown one shows as it is). */
  function modeName(m) {
    const key = `maison.accueil.mode.${String(m).toLowerCase()}`;
    const word = t(key);
    return word === key ? m : word;
  }
  function setMode(m) {
    if (m !== mode) act(`${modeDevice.id}/state`, m, t('maison.accueil.mode_action', { mode: modeName(m) }));
  }

  const talkers = $derived(Object.values(hub.devices).filter((d) => d.points.some((p) => p.key === 'quiet' && p.access?.write)));
  const quiet = $derived(talkers.some((d) => value(d.id, 'quiet') === true));
  const quietUntil = $derived(Math.max(0, ...talkers.map((d) => Number(value(d.id, 'quiet_until') ?? 0))));
  function toggleQuiet() {
    for (const d of talkers) act(`${d.id}/quiet`, !quiet, quiet ? t('maison.accueil.annonces_retablies') : t('maison.accueil.annonces_coupees'));
  }
</script>

{#if modes.length}
  <div class="modes" role="group" aria-label={t('maison.accueil.mode_maison')}>
    {#each modes as m (m)}
      <Tile row icon={MODE_ICON[m] ?? 'home'} label={modeName(m)} on={m === mode} tone="warm" busy={pending(modeDevice.id, 'state') && m !== mode} pressed={m === mode} onclick={() => setMode(m)} />
    {/each}
    {#if talkers.length}
      <Tile
        row
        icon={quiet ? 'volume-off' : 'speaker'}
        label={quiet && quietUntil ? t('maison.accueil.sonos_muette_jusqua', { time: new Date(quietUntil).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' }) }) : t('maison.accueil.sonos_muette')}
        on={quiet}
        tone="alert"
        pressed={quiet}
        onclick={toggleQuiet} />
    {/if}
  </div>
{/if}

<style>
  .modes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--sp-2);
  }
</style>
