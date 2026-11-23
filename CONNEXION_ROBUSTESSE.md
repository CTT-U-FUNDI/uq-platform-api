# Robustesse Connexion

## Cache

Room conserve les réponses:

- offres proches;
- fiche personnelle;
- avis.

La clé de cache inclut le numéro UQ, la méthode et le chemin. Une réponse hors ligne indique qu'elle provient du cache.

## Publication hors ligne

`publish` et `unpublish` peuvent être placés dans une outbox Room. Le bridge renvoie HTTP interne 202 avec un état `queued`.

Ce n'est pas une session financière: aucune commande, aucun paiement ni facturation n'est placée dans cette file.

À la reconnexion, la publication est envoyée. Si l'envoi échoue, l'entrée reste disponible pour une tentative ultérieure.

## Géolocalisation

- consentement visible dans la page;
- permission Android vérifiée;
- précision équilibrée;
- pas de suivi en arrière-plan;
- la position n'est transmise qu'au serveur Connexion;
- les coordonnées précises du vendeur ne sont pas renvoyées dans la vue publique.

## Sécurité réseau

- HTTPS uniquement;
- WebView limitée à l'origine `api.uqplatform.com`;
- fichiers, contenu local, popups et navigation externe bloqués;
- certificat SPKI épinglé;
- JWT et PIN non injectés dans la page;
- CSP stricte dans la page;
- seules les routes Connexion allowlistées sont acceptées.

## Échec et reconnexion

- aucune session inventée;
- aucune offre inventée;
- erreur claire si aucune donnée cache;
- un bouton Actualiser relit le serveur;
- retour réseau déclenche le flush des publications;
- l'achat reste désactivé même hors ou en ligne.

## Tests recommandés

- mode avion: carte, fiche et avis cache;
- publication offline: état 202/queued;
- reconnexion: publication serveur;
- mauvaise origine: requête bloquée;
- route non allowlistée: refus;
- GPS refusé: pas de publication;
- réseau intermittent: aucune session ou paiement créé.
