// ═══════════════════════════════════════════════════════════════
// UQ CHAIN — Passerelle d'Échange (Connectée aux Bulles + 6 Cases)
// ═══════════════════════════════════════════════════════════════

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES
// ═══════════════════════════════════════════════════════════════

pub const EXCHANGE_FEE_RATE: u64 = 1;
pub const EXCHANGE_TIMEOUT_SECONDS: u64 = 3600;
pub const MIN_EXCHANGE_AMOUNT: u64 = 100_000;
pub const MAX_EXCHANGE_AMOUNT: u64 = 100_000_000_000;

// ═══════════════════════════════════════════════════════════════
// TYPES DE PASSERELLES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ExchangeGateway {
    MobileMoney { provider: String, phone_number: String, country: String },
    Crypto { network: String, token: String, wallet_address: String },
    PayPal { email: String },
    BankTransfer { bank_name: String, account_number: String, swift_bic: String, country: String },
    CardPayment { card_type: String, last_four: String },
    UqInternal,
}

impl ExchangeGateway {
    pub fn name(&self) -> &str {
        match self {
            ExchangeGateway::MobileMoney { .. } => "Mobile Money",
            ExchangeGateway::Crypto { .. } => "Crypto",
            ExchangeGateway::PayPal { .. } => "PayPal",
            ExchangeGateway::BankTransfer { .. } => "Virement Bancaire",
            ExchangeGateway::CardPayment { .. } => "Carte Bancaire",
            ExchangeGateway::UqInternal => "Interne UQ",
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// ORDRE D'ÉCHANGE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeOrder {
    pub id: [u8; 32],
    pub user: [u8; 32],
    pub direction: ExchangeDirection,
    pub source: ExchangeGateway,
    pub destination: ExchangeGateway,
    pub source_amount: u64,
    pub BORB_amount: u64,
    pub exchange_rate: f64,
    pub fee: u64,
    pub status: ExchangeStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub external_tx_hash: Option<String>,
    pub bubble_id: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExchangeDirection {
    BORBToExternal,
    ExternalToBORB,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExchangeStatus {
    Pending,
    Confirmed,
    Processing,
    Completed { completed_at: u64 },
    Failed { reason: String, failed_at: u64 },
    Refunded { reason: String, refunded_at: u64 },
    Cancelled { cancelled_at: u64 },
}

// ═══════════════════════════════════════════════════════════════
// MOTEUR D'ÉCHANGE (Connecté aux Bulles)
// ═══════════════════════════════════════════════════════════════

pub struct ExchangeEngine {
    pub orders: HashMap<[u8; 32], ExchangeOrder>,
    pub registry: BubbleRegistry,
    pub rates: HashMap<String, f64>,
    pub total_BORB_sold: u64,
    pub total_BORB_bought: u64,
    pub total_fees_collected: u64,
    pub total_exchanges: u64,
}

impl ExchangeEngine {
    pub fn new(registry: BubbleRegistry) -> Self {
        let mut rates = HashMap::new();
        rates.insert("USD".into(), 1.0);
        rates.insert("EUR".into(), 0.92);
        rates.insert("CDF".into(), 2850.0);
        rates.insert("NGN".into(), 1600.0);
        rates.insert("KES".into(), 129.0);

        ExchangeEngine {
            orders: HashMap::new(),
            registry,
            rates,
            total_BORB_sold: 0,
            total_BORB_bought: 0,
            total_fees_collected: 0,
            total_exchanges: 0,
        }
    }

    /// Vérifie le solde disponible
    pub fn get_balance(&self, owner: &[u8; 32]) -> u64 {
        self.registry.get_balance(owner)
    }

    /// Crée un ordre d'échange (vérifie le solde si ExternalToBORB)
    pub fn create_order(
        &mut self,
        user: &[u8; 32],
        direction: ExchangeDirection,
        source: ExchangeGateway,
        destination: ExchangeGateway,
        source_amount: u64,
        rate: f64,
    ) -> Result<ExchangeOrder, ChainError> {
        let BORB_amount = (source_amount as f64 * rate) as u64;

        if BORB_amount < MIN_EXCHANGE_AMOUNT {
            return Err(ChainError::InternalError(format!(
                "Montant minimum: {} BORB", MIN_EXCHANGE_AMOUNT / 1_000_000
            )));
        }

        if BORB_amount > MAX_EXCHANGE_AMOUNT {
            return Err(ChainError::InternalError(format!(
                "Montant maximum: {} BORB", MAX_EXCHANGE_AMOUNT / 1_000_000
            )));
        }

        // Vérifier le solde si l'utilisateur vend des BORB
        if direction == ExchangeDirection::BORBToExternal {
            let balance = self.registry.get_balance(user);
            if balance < BORB_amount {
                return Err(ChainError::InsufficientFunds("Solde BORB insuffisant".into()));
            }
        }

        let fee = BORB_amount * EXCHANGE_FEE_RATE / 100;
        let now = Self::current_timestamp();

        let order = ExchangeOrder {
            id: Self::generate_order_id(user, now),
            user: *user,
            direction,
            source,
            destination,
            source_amount,
            BORB_amount,
            exchange_rate: rate,
            fee,
            status: ExchangeStatus::Pending,
            created_at: now,
            updated_at: now,
            external_tx_hash: None,
            bubble_id: None,
        };

        self.orders.insert(order.id, order.clone());
        self.total_exchanges += 1;
        Ok(order)
    }

    /// Confirme un ordre
    pub fn confirm_order(&mut self, order_id: &[u8; 32]) -> Result<(), ChainError> {
        let order = self.orders.get_mut(order_id)
            .ok_or_else(|| ChainError::InternalError("Ordre introuvable".into()))?;

        if !matches!(order.status, ExchangeStatus::Pending) {
            return Err(ChainError::InternalError("Ordre déjà traité".into()));
        }

        order.status = ExchangeStatus::Confirmed;
        order.updated_at = Self::current_timestamp();
        Ok(())
    }

    /// Marque un ordre comme complété
    pub fn complete_order(&mut self, order_id: &[u8; 32], external_tx_hash: &str) -> Result<(), ChainError> {
        let order = self.orders.get_mut(order_id)
            .ok_or_else(|| ChainError::InternalError("Ordre introuvable".into()))?;

        if matches!(order.status, ExchangeStatus::Completed { .. }) {
            return Err(ChainError::InternalError("Ordre déjà complété".into()));
        }

        order.status = ExchangeStatus::Completed {
            completed_at: Self::current_timestamp(),
        };
        order.external_tx_hash = Some(external_tx_hash.to_string());
        order.updated_at = Self::current_timestamp();

        match order.direction {
            ExchangeDirection::BORBToExternal => self.total_BORB_sold += order.BORB_amount,
            ExchangeDirection::ExternalToBORB => self.total_BORB_bought += order.BORB_amount,
        }

        self.total_fees_collected += order.fee;
        Ok(())
    }

    /// Annule un ordre
    pub fn cancel_order(&mut self, order_id: &[u8; 32]) -> Result<(), ChainError> {
        let order = self.orders.get_mut(order_id)
            .ok_or_else(|| ChainError::InternalError("Ordre introuvable".into()))?;

        if matches!(order.status, ExchangeStatus::Completed { .. }) {
            return Err(ChainError::InternalError("Ordre déjà complété".into()));
        }

        order.status = ExchangeStatus::Cancelled {
            cancelled_at: Self::current_timestamp(),
        };
        order.updated_at = Self::current_timestamp();
        Ok(())
    }

    /// Met à jour les taux
    pub fn update_rate(&mut self, currency: &str, rate: f64) {
        self.rates.insert(currency.to_string(), rate);
    }

    /// Récupère un taux
    pub fn get_rate(&self, currency: &str) -> Option<f64> {
        self.rates.get(currency).copied()
    }

    /// Calcule le montant BORB
    pub fn calculate_BORB(&self, amount: f64, currency: &str) -> Option<u64> {
        let rate = self.rates.get(currency)?;
        Some((amount * rate * 1_000_000.0) as u64)
    }

    /// Statistiques
    pub fn get_stats(&self) -> ExchangeStats {
        ExchangeStats {
            total_exchanges: self.total_exchanges,
            total_BORB_sold: self.total_BORB_sold,
            total_BORB_bought: self.total_BORB_bought,
            total_fees: self.total_fees_collected,
            pending_orders: self.orders.values().filter(|o| matches!(o.status, ExchangeStatus::Pending)).count() as u64,
            completed_orders: self.orders.values().filter(|o| matches!(o.status, ExchangeStatus::Completed { .. })).count() as u64,
            supported_currencies: self.rates.keys().cloned().collect(),
        }
    }

    /// Détecte un paiement depuis un SMS
    pub fn detect_payment_from_sms(&self, sms_text: &str, country: &str) -> Option<(ExchangeGateway, f64)> {
        if sms_text.contains("M-PESA") || sms_text.contains("MPESA") {
            let amount = Self::extract_amount(sms_text);
            let phone = Self::extract_phone(sms_text);
            return Some((ExchangeGateway::MobileMoney {
                provider: "mpesa".into(),
                phone_number: phone.unwrap(),
                country: country.into(),
            }, amount));
        }
        None
    }

    fn extract_amount(text: &str) -> f64 {
        for word in text.split_whitespace() {
            let cleaned: String = word.chars().filter(|c| c.is_digit(10) || *c == '.').collect();
            if let Ok(amount) = cleaned.parse::<f64>() {
                if amount > 0.0 { return amount; }
            }
        }
        0.0
    }

    fn extract_phone(text: &str) -> Option<String> {
        for word in text.split_whitespace() {
            let cleaned: String = word.chars().filter(|c| c.is_digit(10) || *c == '+').collect();
            if cleaned.len() >= 10 && cleaned.len() <= 15 { return Some(cleaned); }
        }
        None
    }

    fn generate_order_id(user: &[u8; 32], timestamp: u64) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(user);
        hasher.update(&timestamp.to_le_bytes());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }

    fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}

// ═══════════════════════════════════════════════════════════════
// STATISTIQUES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeStats {
    pub total_exchanges: u64,
    pub total_BORB_sold: u64,
    pub total_BORB_bought: u64,
    pub total_fees: u64,
    pub pending_orders: u64,
    pub completed_orders: u64,
    pub supported_currencies: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════
// TESTS (avec UqBlackKey réelle)
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bubble::BubbleRegistry;

    fn create_key() -> UqBlackKey {
        let mut key = UqBlackKey::generate();
        key.activate();
        key
    }

    fn create_registry() -> BubbleRegistry {
        BubbleRegistry::new()
    }

    fn sign_with_key(key: &UqBlackKey, data: &[u8]) -> Vec<u8> {
        key.sign(data)
    }

    #[test]
    fn test_create_exchange_engine() {
        let reg = create_registry();
        let engine = ExchangeEngine::new(reg);
        assert!(engine.rates.contains_key("USD"));
        assert!(engine.rates.contains_key("CDF"));
    }

    #[test]
    fn test_create_order_ok() {
        let reg = create_registry();
        let mut engine = ExchangeEngine::new(reg);
        let user = [1u8; 32];
        let order = engine.create_order(
            &user,
            ExchangeDirection::ExternalToBORB,
            ExchangeGateway::MobileMoney { provider: "mpesa".into(), phone_number: "+254712345678".into(), country: "KE".into() },
            ExchangeGateway::UqInternal,
            200000,
            1.0,
        ).unwrap();
        assert_eq!(order.source_amount, 200000);
        assert!(matches!(order.status, ExchangeStatus::Pending));
        assert!(order.fee > 0);
    }

    #[test]
    fn test_create_order_below_minimum() {
        let reg = create_registry();
        let mut engine = ExchangeEngine::new(reg);
        let user = [1u8; 32];
        assert!(engine.create_order(&user, ExchangeDirection::ExternalToBORB, ExchangeGateway::UqInternal, ExchangeGateway::UqInternal, 100, 1.0).is_err());
    }

    #[test]
    fn test_confirm_and_complete() {
        let reg = create_registry();
        let mut engine = ExchangeEngine::new(reg);
        let user = [1u8; 32];
        let order = engine.create_order(&user, ExchangeDirection::ExternalToBORB, ExchangeGateway::PayPal { email: "user@example.com".into() }, ExchangeGateway::UqInternal, 200000, 1.0).unwrap();
        let order_id = order.id;

        engine.confirm_order(&order_id).unwrap();
        engine.complete_order(&order_id, "TX-123").unwrap();

        let completed = engine.orders.get(&order_id).unwrap();
        assert!(matches!(completed.status, ExchangeStatus::Completed { .. }));
        assert_eq!(engine.total_exchanges, 1);
    }

    #[test]
    fn test_detect_mpesa_sms() {
        let reg = create_registry();
        let engine = ExchangeEngine::new(reg);
        let sms = "M-PESA Confirmed. You have sent KES 500 to John Doe. 254712345678";
        let result = engine.detect_payment_from_sms(sms, "KE");
        assert!(result.is_some());
        let (_, amount) = result.unwrap();
        assert_eq!(amount, 500.0);
    }

    #[test]
    fn test_get_stats() {
        let reg = create_registry();
        let mut engine = ExchangeEngine::new(reg);
        let user = [1u8; 32];
        engine.create_order(&user, ExchangeDirection::ExternalToBORB, ExchangeGateway::UqInternal, ExchangeGateway::UqInternal, 200000, 1.0).unwrap();
        let stats = engine.get_stats();
        assert!(stats.total_exchanges >= 1);
        assert!(stats.pending_orders >= 1);
    }
}

