# Décodeur TV Orange

- Télécommande locale du décodeur (HTTP en clair, port 8080, sans appairage) :
  `GET /remoteControl/cmd?operation=10` donne l'état (veille ou allumé, ce qui est à l'écran,
  l'adresse MAC qui sert d'identité), `operation=01&key=<code>&mode=0` envoie une touche
  (codes d'entrée Linux : 103 haut, 352 OK, 116 marche/veille…).
- La mise en marche est une touche bascule : le pilote ne l'appuie que si le décodeur n'est
  pas déjà dans l'état demandé.
- Points : `power` (lu toutes les 5 s, écrit), `screen` (le contexte affiché : `LIVE`, une
  appli…), `key` (appui seul, mêmes noms que les autres télés : une seule télécommande).
- Vérifié sur un décodeur Sagem (WHD95). Fixtures : réponses réelles, MAC remplacée.
