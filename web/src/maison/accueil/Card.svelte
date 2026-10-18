<script>
  import { optionsOf } from './cards.js';
  import SkyHero from '../ui/SkyHero.svelte';
  import HeatingCard from '../ui/HeatingCard.svelte';
  import MusicCard from '../ui/MusicCard.svelte';
  import Today from '../ui/Today.svelte';
  import EnergyCard from '../ui/EnergyCard.svelte';
  import Routines from '../ui/Routines.svelte';
  import ModesCard from './ModesCard.svelte';
  import NoticesCard from './NoticesCard.svelte';
  import LightsCard from './LightsCard.svelte';
  import ClimateCards from './ClimateCards.svelte';
  import RemoteCard from './RemoteCard.svelte';
  import OutsideCard from './OutsideCard.svelte';
  import DoorbellCard from './DoorbellCard.svelte';
  import RoomSlot from './RoomSlot.svelte';
  import DeviceSlot from './DeviceSlot.svelte';
  import CameraCard from './CameraCard.svelte';
  import PresenceCard from './PresenceCard.svelte';
  import AssistantCard from './AssistantCard.svelte';
  import MachineCard from './MachineCard.svelte';

  /** One card of the home page, by its type: the existing components, with
   *  the card's settings. A type this Moli does not know (a newer one's)
   *  shows nothing. */
  let { card, screen = 'desktop' } = $props();

  const o = $derived(optionsOf(card));
  const title = $derived(card.title ?? null);
  // On the phone a compact card tells less (the sky without its map).
  const compact = $derived(screen === 'phone' && card.size === 's');
</script>

{#if card.type === 'sky'}
  <SkyHero map={o.map !== false && !compact} />
{:else if card.type === 'modes'}
  <ModesCard />
{:else if card.type === 'notices'}
  <NoticesCard allGood={o.allGood !== false} />
{:else if card.type === 'heating'}
  <HeatingCard only={o.only ?? null} next={o.next !== false} planning={o.planning !== false} away={o.away !== false} {title} />
{:else if card.type === 'lights'}
  <LightsCard tiles={o.tiles ?? null} ambiance={o.ambiance !== false} {title} {compact} />
{:else if card.type === 'climate'}
  <ClimateCards devices={o.devices ?? null} {title} />
{:else if card.type === 'remote'}
  <RemoteCard device={o.device ?? null} {title} />
{:else if card.type === 'outside'}
  <OutsideCard facts={o.facts ?? null} {title} />
{:else if card.type === 'doorbell'}
  <DoorbellCard camera={o.camera ?? null} modes={o.modes !== false} {title} />
{:else if card.type === 'music'}
  <MusicCard device={o.device ?? null} {title} />
{:else if card.type === 'today'}
  <Today {title} />
{:else if card.type === 'energy'}
  <EnergyCard {title} />
{:else if card.type === 'routines'}
  <Routines {title} />
{:else if card.type === 'room'}
  <RoomSlot room={o.room ?? null} />
{:else if card.type === 'speaker' || card.type === 'diffuser' || card.type === 'printer'}
  <DeviceSlot kind={card.type} device={o.device ?? null} {title} />
{:else if card.type === 'camera'}
  <CameraCard device={o.device ?? null} refresh={Number(o.refresh) || 15} {title} />
{:else if card.type === 'presence'}
  <PresenceCard {title} />
{:else if card.type === 'assistant'}
  <AssistantCard {title} />
{:else if card.type === 'machine'}
  <MachineCard {title} />
{/if}
