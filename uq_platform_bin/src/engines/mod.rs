// ====================
// 🧠 ENGINES — Bus de moteurs intelligents synchronisés en P2P
// ====================
//
// Quatre moteurs publient des mises à jour signées vers les pairs
// (mesh UDP existant + ingest HTTP pour les appareils connectés) :
//   oracle     — prix FX/BORB/crypto, médiane collective anti-manipulation
//   fraude     — sanctions locales + signalements d'écart oracle
//   reputation — attestations d'activité observée, agrégées par paire
//   synchro    — events métier d'un compte répliqués à SES appareils
//
// Quota : l'état du bus tient dans un budget d'octets par appareil
// (UQ_ENGINE_QUOTA_MB, défaut 100) ; au-delà, purge du plus ancien.
// ====================

pub mod bus;
pub mod gossip;
pub mod handlers;
pub mod rates;
pub mod store;
