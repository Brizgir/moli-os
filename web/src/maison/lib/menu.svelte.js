// The facts the menu's pages are decided on, from every source the
// dashboard already reads (devices, layout, people, Moli's voice): shared by
// the menu (`Maison.svelte`) and its setting (Système, « Pages affichées »).
import { pageFacts } from './home.svelte.js';
import { people } from './people.svelte.js';
import { moli } from './moli.svelte.js';

export const menuFacts = () => pageFacts({ people: { zones: people.zones, phones: people.phones }, voice: !!(moli.status?.speak || moli.status?.listen) });
