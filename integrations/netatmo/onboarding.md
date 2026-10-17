# Onboarding : chauffage Netatmo Energy

1. Prévenir la personne : passage par le cloud de Netatmo (API officielle), avec une app
   qu'elle crée elle-même sur dev.netatmo.com ; sans internet, Moli ne voit plus le chauffage
   (le planning continue de tourner).
2. Déclarer `kind = "netatmo"` (id `netatmo` conseillé ; les secrets sont rangés sous l'id
   de l'instance, ne jamais le renommer) ; `moli-os check-config`, redémarrer. Le pilote
   attend les clés de l'app (« créer une app sur dev.netatmo.com… »).
3. La personne crée l'app pas à pas (README, section 1) ; **Redirect URI** = l'adresse
   affichée par la carte (par défaut `http://localhost/moli-netatmo`).
4. Les clés vont au coffre **par la personne** : carte « Chauffage Netatmo » dans Système (son
   code est demandé), ou `printf %s "$V" | moli-os secrets set netatmo client_id` puis
   `client_secret` par un tube, et redémarrer. **Ne jamais demander le client secret, le code
   ni l'adresse de retour dans le chat.**
5. La personne ouvre la page d'autorisation depuis la carte, accepte, colle l'adresse de
   retour dans Moli. Le statut du pilote passe à `running`.
6. Tester en **lecture d'abord** : pièces chauffées, températures (comparer à l'appli
   Netatmo), mode de la maison, batteries. Puis, la personne présente, **une** consigne sur une
   **pièce neutre** (jamais une chambre ni une pièce protégée), vérifiée dans l'appli Netatmo,
   puis la pièce remise sur `schedule`. Le mode de la maison touche toutes les pièces : pas de
   test sans l'accord de la famille.
7. Vérifier que les pièces Netatmo tombent dans les bonnes pièces Moli (étiquettes) : c'est
   ce qui fait jouer la garde des pièces protégées.
