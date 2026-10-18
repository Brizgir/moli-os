<script>
  import { roomList } from '../lib/home.svelte.js';
  import RoomCard from '../ui/RoomCard.svelte';

  /** A room's card, as on the Pièces page, on the home page. A room that
   *  is no longer there shows nothing. */
  let { room = null } = $props();

  const found = $derived(room ? roomList().find((r) => r.name === room) : null);
</script>

{#if found}
  <RoomCard room={found} />
{/if}
