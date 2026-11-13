// ====================
// 🔮 PRÉDICTIONS — Tendance, volatilité, indicateurs (Phase 2)
// ====================
//
// Volontairement vide en Phase 1 : la prédiction s'appuie sur rates_history,
// qui ne commence à exister qu'après le déploiement du moteur. Prédire sur
// un historique inexistant produirait un chiffre inventé — exactement ce
// qu'on cherche à éliminer.
//
// Une prédiction ne fixe JAMAIS un prix d'exécution : elle est affichée,
// taguée `predictif`, et jamais lue par trading_order_create ni convert_wallets.
// ====================
