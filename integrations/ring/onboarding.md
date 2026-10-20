# Onboarding : Ring

1. Demander : l'e-mail du compte Ring (celui de l'appli), et si la personne a son téléphone
   sous la main (Ring envoie un code de vérification). Prévenir qu'un appareil
   « moli-os/ring » apparaîtra dans le Control Center de l'appli.
2. Déclarer l'instance (aucune option obligatoire) :

   ```toml
   [[driver]]
   id = "ring"
   kind = "ring"
   # [driver.options]
   # history_seconds = 15   # délai maximal d'un coup de sonnette (10 à 3600)
   # siren_seconds = 30     # durée de la sirène (1 à 120)
   ```

3. L'humain range l'e-mail et le mot de passe par un tube, jamais au clavier :
   `printf %s "$E" | moli-os secrets set ring email`, puis `password` ; `moli-os check-config`,
   redémarrer.
4. Le pilote attend (`waiting`) avec la phrase « Ring a envoyé un code » : l'humain range le
   code (`moli-os secrets set ring code_2fa`) et redémarre **dans les minutes qui suivent**.
   Une fois connecté, le mot de passe et le code sont effacés du coffre.
5. Tester, humain présent : sonner à la porte (la sonnette passe à « oui » quelques secondes
   plus tard, puis retombe après 30 s), demander une image. Ne jamais déclencher la sirène
   pour essayer ; le projecteur seulement s'il n'éclaire pas une chambre.
6. Si Ring déconnecte Moli plus tard (mot de passe changé, appareil retiré dans l'appli), le
   pilote attend à nouveau : reposer le mot de passe, redémarrer, redonner le code.
