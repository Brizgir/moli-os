# Créer, renommer et donner d'autres noms aux pièces depuis le tableau de bord

*In English: today the rooms of the family dashboard can only be declared by editing
`data/home.json` by hand (or through an agent). This proposes a small, human-only API
(`POST /api/home/rooms`, `PUT` and `DELETE /api/home/rooms/{name}`) and a « New room » sheet
in the Rooms tab: name, icon, other names; renaming a room carries its devices' labels along
and keeps the old name as an alias. Nothing changes for a house that keeps editing
`home.json` by hand.*

## Le besoin

Les pièces du tableau de bord famille viennent de deux sources : les étiquettes `room` des
appareils (ce que Hue, Tuya ou l'Atelier leur donnent) et `rooms[]` de `data/home.json`
(nom, icône, alias, ordre). Ce second fichier ne s'écrit aujourd'hui qu'à la main ou par un
agent. Dans une maison où personne n'ouvre de terminal, cela laisse trois manques :

- on ne peut pas **créer une pièce vide** pour y ranger ensuite ses appareils (une pièce
  n'existe que si un appareil la nomme) ;
- on ne peut pas **renommer** une pièce sans réétiqueter ses appareils un par un ;
- on ne peut pas dire que « Chambre 2 » (Hue) et « Chambre des enfants » (le plan) sont
  **la même pièce** autrement qu'en éditant `aliases` à la main.

## La proposition

1. **API, session humaine seulement** (même règle que le plan : `x-moli-origin: ui` + cookie
   du code de la maison ; un agent ou un simple en-tête reçoit 403) :
   - `POST /api/home/rooms` `{ name, icon?, aliases? }` : déclare une pièce ;
   - `PUT /api/home/rooms/{name}` (nom ou alias) : change le nom, l'icône, les autres noms ;
   - `DELETE /api/home/rooms/{name}` : retire la déclaration (les appareils gardent leur
     étiquette).
2. **Renommer déplace les appareils** : un nouveau `Hub::rename_room` réécrit l'étiquette
   `room` de chaque appareil rangé dans la pièce (accents et casse ignorés, comme la garde),
   écrit `labels.toml` une fois, émet un `label_changed` et une ligne de journal par appareil.
   **L'ancien nom devient un alias**, pour que ce que Hue ou Alexa appellent encore ainsi
   reste rangé dans la pièce.
3. **Un événement `home_changed`** sur `/api/events` quand `home.json` est écrit : les
   tableaux de bord ouverts relisent la disposition (un téléphone voit la pièce créée sur
   l'ordinateur).
4. **Tableau de bord, onglet Pièces** : un bouton « Nouvelle pièce », une fiche (nom, icône
   parmi une liste fermée, autres noms séparés par des virgules), un ⚙ sur chaque carte pour
   renommer ou retirer. Une pièce déclarée sans appareil s'affiche avec « Aucun appareil ici
   pour l'instant » et un lien vers l'Atelier.

## Choix d'architecture

- **Un seul fichier, le reste intact** : le serveur ne touche qu'à `rooms[]` de `home.json` ;
  `hidden`, `favorites`, `outdoor`… restent tels quels. Écriture atomique (nom temporaire
  unique puis renommage), sérialisée par un verrou : deux onglets ne se marchent pas dessus.
- **Bornes** : nom nettoyé (espaces simples, 60 caractères au plus), pas de doublon (nom ou
  alias d'une autre pièce, accents et casse ignorés), icône dans une liste fermée
  (`ROOM_ICONS`, chacune présente dans `lib/icons.js`, vérifié par un test), 10 alias et
  100 pièces au plus. Erreurs en 422 avec un message traduit.
- **Protéger une pièce reste dans `moli.toml`** (`[guard] protected_rooms`, lu au démarrage) :
  la fiche le dit, ne le propose pas. Pas de nouvelle surface pour la garde.
- **Pas de LLM, pas d'agent** : c'est une décision de personne, comme arranger le plan.
- `same_room` de la garde devient `pub` pour être réutilisé par `rename_room` (même
  comparaison partout).

## Ce qui ne change pas pour les autres maisons

- Une maison qui écrit `home.json` à la main continue : le format est le même, le serveur ne
  réécrit le fichier que quand une personne crée, renomme ou retire une pièce.
- Sans `home.json`, rien ne change à l'affichage (pièces telles que les appareils les
  nomment) ; la première pièce créée crée le fichier.
- Aucun pilote n'est touché ; aucune nouvelle dépendance.

Je peux proposer la pull request correspondante (une branche, un commit, tests HTTP et du
hub compris).
