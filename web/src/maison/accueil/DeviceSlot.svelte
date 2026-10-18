<script>
  import { device, nameOf } from '../lib/home.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import SpeakerCard from '../ui/SpeakerCard.svelte';
  import DiffuserCard from '../ui/DiffuserCard.svelte';
  import PrinterCard from '../ui/PrinterCard.svelte';

  /** One device on the home page: a speaker, a diffuser, a printer, with
   *  its own card. Gone from the house: nothing. */
  let { kind, device: id = null, title = null } = $props();

  const found = $derived(id ? device(id) : null);
</script>

{#if found && kind === 'speaker'}
  <SpeakerCard {id} name={title} />
{:else if found && kind === 'printer'}
  <PrinterCard {id} />
{:else if found && kind === 'diffuser'}
  <section class="card">
    <div class="card-head"><h2><Icon name="fan" size={18} />{title ?? nameOf(id)}</h2></div>
    <DiffuserCard {id} />
  </section>
{/if}
