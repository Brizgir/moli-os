// The automations, read for what the day holds (the heating's next change,
// the « Aujourd'hui » card): one request every two minutes, however many
// cards ask. What they compute is in `today.js`.
import { api } from './auto.svelte.js';

export const timed = $state({ list: [] });

let timer = null;
let users = 0;

async function refresh() {
  try {
    const { automations } = await api.list();
    timed.list = automations ?? [];
  } catch {
    /* a guest, or the engine off: nothing timed to tell */
  }
}

/** Starts reading while a card needs it. Returns a stop function. */
export function useTimed() {
  users += 1;
  if (users === 1) {
    refresh();
    timer = setInterval(refresh, 120_000);
  }
  return () => {
    users -= 1;
    if (users === 0) clearInterval(timer);
  };
}
