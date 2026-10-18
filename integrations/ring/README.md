# Sonnettes et carillons Ring

Moli lit les sonnettes, caméras et carillons d'un compte Ring : quand on sonne, quand ça bouge
devant la porte, la batterie, le signal Wi-Fi, le volume du carillon, la dernière image de chaque
caméra quand le tableau de bord la montre, et le mode de surveillance du lieu (Désactivé,
Domicile, Absent, comme les « Modes » de l'appli Ring), qu'une personne change depuis le
tableau de bord. Un appui peut lancer un automatisme comme n'importe quel bouton (une annonce
sur les enceintes, une lumière…).

**À savoir avant de l'installer.** Les appareils Ring ne répondent à rien sur le réseau de la
maison : tout passe par les serveurs de Ring, et Ring ne propose aucune interface officielle.
Moli utilise celle de l'appli Ring, comme le font Home Assistant (python-ring-doorbell) et
ring-client-api (Homebridge) : elle peut changer ou cesser sans préavis.

## Connexion

Le mot de passe Ring ne passe **jamais** par Moli. Ring n'offre pas de page de connexion pour
une autre application : le jeton se fait sur l'ordinateur de la personne, avec le client de
Ring lui-même, puis se colle dans Moli.

1. Sur un ordinateur où Node.js est installé : `npx -p ring-client-api ring-auth-cli`. Ring y
   demande l'e-mail, le mot de passe et le code de vérification (SMS, e-mail ou appli) ; la
   commande affiche à la fin un jeton (une longue ligne).
2. Depuis le tableau de bord (Système, carte Ring), coller ce jeton. Moli l'envoie une fois à
   Ring pour le vérifier ; Ring le remplace aussitôt par un nouveau, que Moli garde chiffré
   dans son coffre et renouvelle lui-même ensuite. Ce qui a été collé n'est ni affiché, ni
   journalisé, ni gardé.

Moli apparaît alors dans le Centre de contrôle de l'appli Ring, parmi les appareils clients
autorisés (le même que l'ordinateur qui a fait le jeton) : on peut l'y retirer, et il faudra
refaire un jeton.

## Configuration

```toml
[[driver]]
id = "ring"
kind = "ring"
[driver.options]
# poll = 10   # secondes entre deux lectures des derniers événements (5 au moins)
```

## Ce que Moli voit

| Appareil | Point | Sens |
|---|---|---|
| sonnette | `doorbell` | vrai pendant 10 s après un appui |
| sonnette, caméra | `motion` | vrai pendant 60 s après le dernier mouvement vu par Ring |
| sonnette, caméra | `battery` | % (appareils sur batterie) |
| sonnette, caméra | `image_at` | quand l'image montrée a été prise (Ring peut redonner une ancienne) |
| tous | `wifi_signal` | dBm |
| carillon | `volume` | 0 à 11, en lecture |
| Surveillance Ring (le lieu) | `mode` | `disarmed`, `home`, `away` ; **écrivable**, par une personne |
| Surveillance Ring | `mode_since` | quand le mode a changé pour la dernière fois |

## Images

Chaque sonnette et caméra apparaît dans l'onglet Caméras, et la sonnette sur l'accueil, avec la
dernière image que Ring en a. Une image fraîche se demande à Ring et prend quelques secondes ;
une caméra sur batterie se réveille pour ça, et n'en donne pas pendant qu'elle enregistre. Moli
ménage la batterie et les quotas de Ring : une image n'est demandée que quand le tableau de bord
l'affiche (jamais en boucle), une seule à la fois par caméra, pas plus d'une par minute sur
batterie (quinze secondes sur secteur) ; entre deux, l'image gardée est montrée avec **son**
horodatage (`image_at`), jamais présentée comme neuve si Ring redonne la même. Une caméra dans
une pièce protégée reste réservée à une personne (le code de la maison), comme les autres.

## Modes de surveillance

Le lieu Ring apparaît comme un appareil « Surveillance Ring », avec les trois modes de l'appli :
**Désactivé**, **Domicile**, **Absent**. Le mode est relu toutes les 30 s (un changement fait
dans l'appli Ring se voit dans Moli en moins d'une minute) et juste après chaque ordre : ce que
Moli montre est ce que Ring dit. Trois boutons, sur la page « Sonnette et caméras » et sur
l'accueil, le mode actuel en évidence.

Changer le mode est l'affaire d'une personne : depuis le tableau de bord, le **code de la
maison** est demandé à l'appui. Moli (l'assistant), un agent MCP, un script ou le tableau de bord
sans code n'obtiennent qu'une demande d'approbation ; un automatisme qu'une personne a validé
peut le faire (c'est son intention). Un lieu avec une alarme Ring garde son mode dans l'appli Ring
(Moli le lit, ne le change pas).

En ligne ou non : la disponibilité de chaque appareil, telle que Ring la donne ; quand les
serveurs de Ring sont injoignables, tous les appareils passent indisponibles et `doorbell`
et `motion` retombent à faux.

## Limites

- **Pas instantané** : Moli relit les derniers événements de chaque sonnette toutes les 10 s (les
  notifications push de l'appli seront une suite). Un appui est donc vu 10 à 20 s après. Un
  événement de plus de 90 s n'est jamais annoncé, ni rien de ce qui s'est passé avant le
  démarrage de Moli : jamais de « on sonne » périmé.
- Hors le mode de surveillance, rien n'est commandé (volume du carillon, sirène, lumière : une
  suite).
- Pas de vidéo en direct : une image à la demande, pas plus d'une par minute sur batterie.
- Un changement de mot de passe Ring, ou Moli retiré du Centre de contrôle, oblige à refaire
  un jeton.
