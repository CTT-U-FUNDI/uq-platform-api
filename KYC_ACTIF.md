# KYC actif

## Version retenue

Le KYC actif est le parcours V2 guidé porté dans la V3:

- `src/app/_hors_v3/screens/KycScreen.tsx`
- `src/app/_hors_v3/components/CaptureStep.tsx`
- `src/app/_hors_v3/components/CameraStage.tsx`
- `src/v2/lib/kycLive.ts`

Il est plus complet que `features/kyc/KycFlow`, qui simulait plusieurs résultats et affichait un score fixe de 89 %.

## Parcours

1. préparation et état du dossier;
2. nom, prénom, date, pays, adresse, téléphone, numéro de pièce;
3. capture guidée du document;
4. selfie guidé;
5. session de vivacité à usage unique;
6. envoi réel dans la file KYC.

## Routes réelles

- `POST /api/kyc/live/session`
- `POST /api/kyc/live/face`
- `POST /api/kyc/live/doc`
- `POST /api/kyc/live/check`
- `POST /api/core/verify-identity`
- `GET /api/kyc/statut/{user}`
- `GET /api/kyc/status`

## Sécurité

`POST /api/account/verification/submit` ne soumet et ne valide plus un dossier.

Il exige désormais:

- JWT valide;
- compte ciblé identique au JWT, sauf autorité owner;
- statut serveur KYC `Verifie`;
- cadenas structurel valide.

Sans ces conditions, le déverrouillage répond `KYC_NON_VALIDE`.

## Statut automatique

La détection Tesseract ne dépend plus seulement du chemin Windows. Elle accepte Windows et Linux Render et vérifie aussi la commande disponible dans `PATH`.

## Limites

- La validation finale reste manuelle dans la file owner.
- Les anciens composants KYC décoratifs restent hors du parcours actif.
- `/api/kyc/doc/analyze`, `/face/compare` et `/liveness` utilisent des chemins serveur et ne remplacent pas le flux live mobile.
