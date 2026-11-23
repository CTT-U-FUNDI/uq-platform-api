# Escrow P2P — sécurité implémentée

## Objectif

Renforcer le flux P2P BORB sans simuler de paiement ni de libération.

## Moyens de réception

Routes:

- `GET /api/p2p/receiving-methods/{uq}`
- `POST /api/p2p/receiving-methods`
- `POST /api/p2p/receiving-methods/{id}/verify`

Une offre de vente doit sélectionner un moyen vérifié. Le moyen est figé dans la commande. Les réponses publiques exposent seulement type, opérateur, devise et quatre derniers chiffres. Le nom du titulaire n'est jamais renvoyé.

## Défi aveugle

Route:

- `POST /api/p2p/market/orders/{id}/name-challenge`

L'acheteur saisit le nom réellement affiché par son opérateur mobile money. Le serveur compare un hash normalisé. Trois échecs verrouillent le défi. Une preuve ne peut pas être déposée avant validation du défi.

## Codes croisés

Routes:

- `POST /api/p2p/market/orders/{id}/codes/rotate`
- `POST /api/p2p/market/orders/{id}/codes/verify`

À la création:

- cinq codes différents sont créés;
- un seul code est actif;
- les codes sont stockés hashés;
- l'expiration est de 30 minutes;
- le vendeur reçoit le code actif uniquement dans la notification privée.

La rotation exige le PIN vendeur. Le code de l'acheteur est utilisable une seule fois.

## Preuves

- SHA-256 obligatoire pour chaque image;
- réutilisation exacte d'une image refusée sur toute transaction;
- cinq preuves maximum par participant et commande;
- nom, montant, devise, numéro et analyse serveur conservés;
- le nom du bénéficiaire attendu n'est jamais exposé.

## Libération

La route `POST /api/p2p/market/orders/{id}/complete` exige désormais:

- participant exact, sans substitution owner;
- PIN vendeur configuré et valide;
- version attendue de la commande;
- clé d'idempotence;
- défi de nom validé;
- code croisé validé;
- statut `paid`;
- journal de règlement.

Une reprise d'appel avec la même clé retrouve la réponse déjà enregistrée. Une variation de payload sous la même clé est refusée.

## Litiges

Routes:

- `POST /api/p2p/market/orders/{id}/dispute`
- `POST /api/p2p/market/orders/{id}/arbitrate`

Après paiement, annuler devient un litige et ne dégage plus les BORB. En litige, seuls les arbitres peuvent décider:

- `release`: libération à l'acheteur;
- `refund`: dégel/remise à la disposition du vendeur.

## Journal de règlement

Le module `p2p_escrow` écrit un journal dans `uq_index.db`. En cas de résultat partiel entre deux bases comptes, la reprise détecte les écritures déjà présentes et ne termine que la jambe manquante. Une commande déjà terminée est idempotente.

## Limites encore à traiter

- OCR serveur automatique de la preuve non encore branché;
- hash perceptuel des images redimensionnées non encore ajouté;
- audit cryptographique lié entre événements non encore finalisé;
- stockage binaire des preuves chiffré hors SQLite non encore migré;
- seuils de montant/KYC et caméra UQ non encore déployés.

Ces limites ne sont pas simulées: elles continuent à être signalées comme indisponibles ou soumises au contrôle backend existant.

## Tests

Tests ciblés:

- `p2p_security::tests`
- `p2p_preuve::tests::image_reutilisee_est_refusee_sur_une_autre_preuve`
