**Titre proposé :** Plannings de chauffe Netatmo lus et modifiés depuis Moli (et un « service » d'appareil générique)

*Dépend de l'issue « Pilote natif Netatmo Energy ».*

---

## Le besoin

Avec le pilote Netatmo Energy, Moli lit et règle les pièces, mais le cœur du chauffage reste
**le planning de la semaine** : les heures de Confort, Nuit, Éco et leurs températures par
pièce. Aujourd'hui il faut l'appli Netatmo pour le voir ou le changer. On voudrait le faire
depuis le tableau de bord de Moli, à côté du reste de la maison, sans donner ce pouvoir à un
agent ni à un automatisme.

## La proposition

1. **Un « service » par appareil, générique** (`moli_runtime::service`) : ce qu'un appareil
   sait faire au-delà de ses points, trop structuré pour une valeur — un document à lire et
   des actions dessus. Un pilote l'enregistre (`DriverCtx::provide_service`), le hub l'appelle
   sans file (25 s au plus), chaque action est journalisée (`Action::Service`).
   - `GET /api/devices/{id}/service/{nom}` : lecture, libre comme `/api/devices` (un invité :
     ses pièces seulement) ;
   - `POST /api/devices/{id}/service/{action}` : **une personne au tableau de bord avec son
     code** (`x-moli-origin: ui` + session humaine), jamais un agent, un script, un automatisme
     ni un invité — comme `PUT /api/plan` ; 10 écritures / 10 s.
2. **Le pilote Netatmo l'utilise** pour les plannings : lecture (`schedules`), modification
   (`sync` → `synchomeschedule`), création (`create` → `createnewhomeschedule`), choix du
   planning suivi (`activate` → `switchhomeschedule`). Un point `schedule` sur la maison (le
   planning suivi, sous la garde comme toute commande) et `setpoint_end` sur chaque pièce (fin
   de la consigne manuelle).
3. **Une page « Planning de chauffe »** (`#/chauffage`) : la semaine en grille, glisser une
   limite, toucher une plage pour changer son réglage, recopier un jour sur d'autres,
   températures par réglage et par pièce, absence et hors-gel, planning suivi, « Dupliquer
   pour essayer ». En haut, ce qui passe avant le planning (consigne manuelle en cours avec
   « Revenir au planning », maison en absence ou hors-gel).

## Choix d'architecture

- **Le service plutôt qu'un point** : un planning (zones, températures par pièce, créneaux)
  n'est pas une valeur ; le forcer dans un point aurait tordu le modèle. Le service est petit
  (un trait, deux méthodes) et réutilisable par d'autres pilotes (programmes d'une machine,
  scènes d'un pont…).
- **Validation stricte avant tout envoi** (`Draft::parse`) : nom 1–60, températures 7–30 par
  0,5, zones distinctes et connues, chaque pièce chauffée dans chaque zone, ≤ 20 zones,
  ≤ 1 000 créneaux, le premier à lundi 0 h, strictement croissants, < 10 080 minutes. Rien
  d'invalide ne part chez Netatmo.
- **Une requête à la fois**, par la boucle du pilote (mêmes quotas et espacement que le
  reste), puis `homesdata` relu : la réponse est ce que Netatmo garde, pas ce qu'on a envoyé.
- La logique de la semaine côté web est pure et testée (`web/src/maison/lib/heating.js`,
  `node --test`).

## Sécurité

- Écrire un planning est un geste humain seulement : ni la garde ni une approbation ne peuvent
  le donner à un agent. Le code de la personne est demandé pour enregistrer.
- Changer le planning **suivi** par le point `schedule` passe par la garde : la maison a toutes
  ses pièces pour membres, donc l'ordre d'un agent est retenu dès qu'une pièce protégée en fait
  partie.
- Moli ne supprime aucun planning.

## Ce qui ne change pas pour les autres maisons

Le service est un ajout : aucun pilote existant ne change, aucune route existante ne change de
sens. La page « Planning de chauffe » et son lien n'apparaissent qu'avec une maison Netatmo.

---

*In English: read and edit Netatmo heating schedules from Moli's dashboard, through a small
generic per-device "service" (a document to read, actions on it). Reading is free; writing
is a person's gesture only (dashboard + their code), never an agent's. Strict validation
before anything is sent, one request at a time, the home re-read afterwards. Nothing changes
for homes without Netatmo.*
