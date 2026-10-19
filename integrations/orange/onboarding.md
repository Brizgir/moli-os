# Onboarding : décodeur TV Orange

1. Trouver son adresse : la box la liste (« Décodeur TV Orange »), ou une recherche SSDP
   (`X_OrangeSTBRemoteControl`). Lui donner une adresse fixe dans la box si possible.
2. Aucun identifiant, aucun appairage. Vérifier qu'il répond :
   `curl "http://<adresse>:8080/remoteControl/cmd?operation=10"`.
3. Déclarer `kind = "orange"` avec `host` ; `moli-os check-config`, redémarrer.
4. Tester : `power` suit la veille, une touche (`key` = `CursorUp`) agit à l'écran.
5. Le ranger dans sa pièce : sa télécommande apparaît dans la page de la pièce.
