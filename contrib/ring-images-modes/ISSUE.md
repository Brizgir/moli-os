# Ring : images des caméras à la demande et mode de surveillance Désactivé / Domicile / Absent

*In English: follows the read-only Ring driver. Each Ring doorbell and camera gives its last
image on demand (only when a dashboard shows it, never in a loop, at most one per minute on
battery), shown with the time the camera took it. Each Ring location becomes a « Ring watch »
device whose `mode` (disarmed, home, away) is the only thing Moli ever writes to Ring; the
point is `Semantic::Control`, so the guard holds any order that does not come from a person
with the code (`ui`), a validated automation or the system. Nothing changes for homes without
Ring.*

Dépend de l'issue et de la PR « Pilote natif Ring » (#…).

## Le besoin

Avec le pilote Ring en lecture, Moli sait qu'on sonne mais ne montre pas **qui** est à la porte,
et la surveillance Ring (les « Modes » de l'appli : Désactivé, Domicile, Absent) se change
seulement dans l'appli Ring. On voudrait :

- voir la dernière image de la sonnette et des caméras dans l'onglet Caméras et sur l'accueil ;
- changer le mode de surveillance depuis le tableau de bord de la maison, sans que cela devienne
  quelque chose qu'un agent, un script ou un tableau de bord sans code puisse faire seul.

## La proposition

**Images.** Chaque sonnette et caméra Ring déclare au hub une source d'images
(`SnapshotSource`, déjà utilisée par d'autres caméras) : `/api/devices/<id>/snapshot` marche
pour elles comme pour les autres caméras. Une image fraîche se demande à Ring (`PUT
snapshots/update`, `POST snapshots/timestamps` jusqu'à un horodatage plus récent, `GET
snapshots/image/<id>`).

**Modes.** Un appareil « Surveillance Ring » par lieu Ring, point `mode`
(`disarmed|home|away`, écrivable) et `mode_since`, lu toutes les 30 s et juste après chaque
ordre (`GET/POST /api/v1/mode/location/<lieu>` sur l'hôte de l'appli). Trois boutons
(`RingModes.svelte`) sur la page Caméras et sur l'accueil.

## Choix d'architecture

- **Images à la demande seulement, jamais en boucle** : une caméra sur batterie se réveille
  pour chaque image, et Ring a des quotas. Une image n'est demandée que quand une surface
  l'affiche, une à la fois par caméra (verrou du hub et cache du pilote), au plus une par 60 s
  sur batterie (15 s sur secteur) ; entre deux, l'image gardée.
- **L'heure de l'image est celle de la caméra** (point `image_at`) : Ring peut redonner une
  image ancienne ; elle n'est jamais présentée comme neuve. La tuile caméra montre cette heure
  plutôt que l'heure de lecture.
- **Ce que Moli montre est ce que Ring dit** : après un ordre de mode, Moli relit le mode ;
  l'ordre est en erreur si Ring ne montre pas le mode demandé. Un changement fait dans l'appli
  est vu en moins d'une minute.
- **Le seul point écrit chez Ring est le mode** : volume du carillon, sirène, lumière restent
  hors de portée.
- Petite addition au runtime : `SecretKeeper`, pour ranger au coffre un jeton renouvelé par une
  demande d'image (hors de la boucle du pilote). Le client Ring est partagé entre la boucle et
  les images (`Arc<Mutex<Client>>`).

## Sécurité

- **La garde du mode** : le point `mode` est `Semantic::Control`, la sémantique déjà utilisée
  pour « pause / annuler » d'une imprimante. La garde le retient pour un humain **quelle que
  soit la pièce** dès que l'origine n'est pas `ui` (une session avec le code), `automation` (un
  automatisme qu'une personne a validé) ou `system` : `household` (le tableau de bord sans
  code), `assistant`, `mcp`, `api`, `cli` reçoivent une demande d'approbation (202). Armer ou
  désarmer la surveillance d'une maison habitée est l'affaire d'une personne. Seul le libellé
  `serveur.garde.machine` change (« ordre réservé à une personne (machine au travail, mode de
  surveillance) »), la règle de la garde reste celle de `main`.
- Un lieu avec une **alarme Ring** (`readOnly`) est lu, jamais écrit.
- Les images d'une caméra dans une **pièce protégée** restent réservées à une personne, comme
  pour toutes les caméras (rien de propre à Ring).
- **Interface non officielle**, comme pour la lecture : les adresses des instantanés et des modes
  viennent des projets ouverts (ring-client-api, python-ring-doorbell) ; elles peuvent changer.

## Ce qui ne change pas pour les autres maisons

- Rien sans pilote Ring : `RingModes` ne s'affiche que pour un appareil Ring qui a un point
  `mode` écrivable ; l'accueil et la page Caméras sont identiques sans lui.
- Deux retouches génériques de l'interface, sans effet pour une maison sans Ring : sur la page
  Caméras, une caméra qui a un point `battery` n'est demandée qu'au toucher (comme une caméra
  favorite marquée `battery`) ; sur l'accueil, **sans caméra favorite**, la carte de la porte
  prend une sonnette qui donne une image. Une disposition qui a ses caméras favorites ne change
  pas. La tuile caméra montre l'heure `image_at` seulement si l'appareil a ce point.
- La garde, les approbations et le journal ne changent pas ; le test de la garde couvre en plus
  `mcp`, `household` et `api` pour un point `Control`.

## Vérifié

Sur une installation réelle (octobre 2026, une sonnette sur batterie et deux Stick Up Cam
Mini) : image de la sonnette obtenue ; changement de mode depuis Moli vu dans l'appli Ring. Les
Stick Up Cam Mini ne donnent une image qu'avec les « Instantanés » activés pour elles dans
l'appli Ring (documenté dans le README et l'onboarding).
