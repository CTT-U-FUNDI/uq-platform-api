# Boutique Connexion

## Résultat

La boutique Connexion est accessible depuis la carte système du flux Zando et s'ouvre dans une activité Android native avec:

- en-tête Jetpack Compose;
- bouton Retour;
- indicateur réseau natif;
- WebView limitée à `https://api.uqplatform.com`;
- quatre sections: Acheter, Vendre, Mes offres, Confiance;
- cache Room des dernières réponses;
- file d'attente des publications en perte de réseau.

## Architecture

- KMP partagé: `uq_frontend/android/connexion-shared`
- UI Compose/WebView: `uq_frontend/android/connexion-ui`
- plugin Capacitor: `UqConnexionPlugin`
- carte système: `uq_frontend/src/app/screens/HomeScreen.tsx`
- interface servie par Render: `deploy-api/uq_platform_bin/static/connexion`
- bridge natif: WebMessage, origine exacte, routes allowlistées
- réseau: OkHttp avec certificat SPKI épinglé
- token: mémoire native uniquement, jamais envoyé au JavaScript ni stocké dans une URL

## Routes utilisées

Opérationnelles:

- `GET /api/connexion/nearby`
- `GET /api/connexion/mine`
- `POST /api/connexion/publish`
- `POST /api/connexion/unpublish`
- `POST /api/connexion/avis`
- `GET /api/connexion/avis?annonce_id=...`

Non disponibles et non appelées:

- création d'une session Connexion;
- facturation à la seconde;
- accord tarif/client/vendeur;
- début ou fin de session;
- temps facturable;
- débit mesuré côté serveur;
- gel/débit/crédit;
- litige session;
- rapprochement financier;
- `GET /api/connexion/router/script`;
- liste serveur de routeurs compatibles;
- activation native d'un point d'accès Android.

## Blocage d'achat

Le bouton Acheter la connexion est toujours désactivé. Message affiché:

> La facturation à la seconde n'est pas encore disponible. Reviens bientôt.

Les prix par seconde ne sont pas publiés ni récupérés: le backend actuel les refuse explicitement.

## Vendre

La fiche accepte uniquement les données que le backend publie:

- nom;
- réseau mobile ou satellite;
- débit estimé;
- latence;
- signal;
- position consentie;
- horodatage de mesure.

La WebView n'accepte ni mot de passe routeur ni script de configuration. Aucun point d'accès téléphonique n'est activé.

## Confiance

Les avis backend sont affichés. Aucun score de confiance Connexion sur 100, nombre de sessions, taux de réussite ou revenus n'est inventé, car les sessions ne sont pas implémentées.

## Validation

- Build web Vite réussi.
- Module `connexion-ui` compilé.
- APK debug complet assemblé.
- Tests KMP `ConnexionModelsTest` réussis.
- Les routeurs et l'activation hotspot restent indisponibles par conception.

## Vérification sur appareil

À effectuer sur téléphone ou émulateur:

1. ouvrir la carte Boutique Connexion;
2. vérifier le retour vers le flux;
3. accorder/refuser la localisation;
4. publier une fiche;
5. consulter/depublier la fiche;
6. couper Wi-Fi/données et vérifier le cache;
7. revenir en ligne et vérifier le flush de publication;
8. confirmer que Acheter reste désactivé.
