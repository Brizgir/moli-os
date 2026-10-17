# Sonnettes et carillons Ring

Moli lit les sonnettes, caméras et carillons d'un compte Ring : quand on sonne, quand ça bouge
devant la porte, la batterie, le signal Wi-Fi, le volume du carillon. Un appui peut lancer un
automatisme comme n'importe quel bouton (une annonce sur les enceintes, une lumière…).

**À savoir avant de l'installer.** Les appareils Ring ne répondent à rien sur le réseau de la
maison : tout passe par les serveurs de Ring, et Ring ne propose aucune interface officielle.
Moli utilise celle de l'appli Ring, comme le font Home Assistant (python-ring-doorbell) et
ring-client-api (Homebridge) : elle peut changer ou cesser sans préavis.

## Connexion

Depuis le tableau de bord (Système, carte Ring) : l'e-mail et le mot de passe du compte Ring,
puis le code de vérification que Ring envoie (SMS, e-mail ou appli d'authentification). Moli
les transmet à Ring et ne les garde pas : ni fichier, ni coffre, ni journal (ils n'existent
qu'en mémoire le temps de la saisie du code, 10 minutes au plus). Il garde seulement le jeton
que Ring lui donne, chiffré dans son coffre, et le remplace à chaque fois que Ring en donne un
nouveau. Moli apparaît alors dans le Centre de contrôle de l'appli Ring, parmi les appareils
clients autorisés : on peut l'y retirer, et il faudra se reconnecter.

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
| tous | `wifi_signal` | dBm |
| carillon | `volume` | 0 à 11, en lecture |

En ligne ou non : la disponibilité de chaque appareil, telle que Ring la donne.

## Limites

- **Pas instantané** : Moli relit les derniers événements de chaque sonnette toutes les 10 s (les
  notifications push de l'appli seront une suite). Un appui est donc vu 10 à 20 s après. Un
  événement de plus de 90 s n'est jamais annoncé, ni rien de ce qui s'est passé avant le
  démarrage de Moli : jamais de « on sonne » périmé.
- Lecture seule : rien n'est commandé (volume du carillon, sirène, lumière : une suite).
- Pas d'image pour l'instant (l'instantané Ring est une suite).
- Un changement de mot de passe Ring, ou Moli retiré du Centre de contrôle, oblige à refaire la
  connexion.
