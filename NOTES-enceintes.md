# Enceintes pilotables depuis Pièces : où on en est

Fichier de passage de relais, à supprimer avant la fusion.

## Fait

- `web/src/maison/lib/home.svelte.js` : `deviceKind` classe `speaker` un appareil avec `playing` ✎ et
  (`volume` ou `play` ✎), donc les groupes Alexa (sans volume) aussi. Nouveaux : `speakerState(id)`
  (`playing` / `paused` pour `paused` Alexa et `paused_playback` Sonos / `stopped`), `speakerGroup(d)`
  (enceinte sans volume), `roamingSpeakers()` (enceintes et groupes sans pièce).
- `ui/SpeakerCard.svelte` réécrite : prop `inline` (dans la carte de pièce), état, titre/artiste,
  lecture/pause, volume et sourdine seulement si les points existent, champ « Mets… » (`play`) /
  « Dis… » (`say`, 250 car.) avec bascule si les deux existent, cibles ≥ 44 px. « Le son de la télé »
  seulement pour `home.json` `salon.speaker` ; sinon « Ça joue ». Icône `speaker-multiple` pour un
  groupe (ajoutée dans `lib/icons.js`).
- `ui/RoomCard.svelte` : la pastille d'une enceinte n'envoie plus vers `#/salon` ; un appui ouvre la
  `SpeakerCard` en dessous (comme le ⚙ d'une lampe), une à la fois ; état « En lecture / En pause /
  À l'arrêt » ; `room.note` remplace le sous-titre si présent.
- `pages/Pieces.svelte` : carte « Toute la maison » en bas pour les enceintes sans pièce (groupes
  « Partout », « Maison »), soumise aux filtres.
- Traductions : `commun.enceinte.*` (fr, en), `maison.pieces.toute_la_maison(_note)` ; supprimées
  `salon.enceinte.*` et `commun.piece.lecture_en_cours` / `a_l_arret` (plus utilisées).
- Doc : `docs/components/dashboard.md`, ligne Pièces.

## Vérifié

- `node scripts/i18n-check.mjs` : OK (dans `node:24-alpine`, pas de node local).
- `npm run build` : OK.
- `scripts/check.sh` : `cargo fmt` et `clippy` passés ; **tests, catalogue et leak-check pas allés
  au bout** (arrêté pour soulager la machine). Aucun code Rust touché.

## Reste à faire

1. Relancer `sh scripts/check.sh` jusqu'au bout.
2. Essai visuel : une copie de `demo/villa` avec un faux pilote Alexa (deux Echo au Salon, un groupe
   « Partout » sans pièce, fixture `imitates: "sonos"` car `alexa` n'est pas imitable) ; vérifier la
   carte en largeur téléphone, la bascule Mets/Dis, la carte « Toute la maison ».
3. Commit propre (retirer ce fichier), puis PR.

## Suites repérées (hors périmètre)

- `pages/Plan.svelte` (~ligne 199) : un appui sur une enceinte du plan envoie encore vers `#/salon`.
- Garde : un groupe Alexa « Partout » n'a pas de pièce, donc une commande sur lui n'est pas retenue,
  alors qu'il joue aussi sur l'Echo d'une pièce protégée. À traiter côté serveur (pilote ou garde).
