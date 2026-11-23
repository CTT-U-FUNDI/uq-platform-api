use std::sync::Mutex;
use uq_blockchain::bubble::BubbleRegistry;
use uq_blockchain::exchange::{ExchangeDirection, ExchangeEngine, ExchangeGateway};
use uq_blockchain::insurance::InsuranceFund;
use uq_blockchain::lending::LendingPool;

/// État global pour les services financiers
pub struct ServiceState {
    pub exchange: Mutex<ExchangeEngine>,
    pub lending: Mutex<LendingPool>,
    pub insurance: Mutex<InsuranceFund>,
}

impl ServiceState {
    pub fn new(registry: BubbleRegistry) -> Self {
        Self {
            exchange: Mutex::new(ExchangeEngine::new(registry)),
            lending: Mutex::new(LendingPool::new()),
            insurance: Mutex::new(InsuranceFund::new()),
        }
    }

    /// Crée un ordre d'échange
    pub fn create_exchange_order(
        &self,
        user_hex: &str,
        direction: &str,
        amount: u64,
    ) -> Result<String, String> {
        let mut engine = self.exchange.lock().unwrap();

        let user_bytes = hex::decode(user_hex).map_err(|_| "Adresse invalide")?;
        let mut user = [0u8; 32];
        user.copy_from_slice(&user_bytes[..32]);

        let dir = match direction {
            "to_external" => ExchangeDirection::BORBToExternal,
            "to_BORB" => ExchangeDirection::ExternalToBORB,
            _ => return Err("Direction invalide".into()),
        };

        // Gateway par défaut : MobileMoney
        let source = if dir == ExchangeDirection::ExternalToBORB {
            ExchangeGateway::MobileMoney {
                provider: "MTN".to_string(),
                phone_number: "+243000000000".to_string(),
                country: "CD".to_string(),
            }
        } else {
            ExchangeGateway::UqInternal
        };

        let destination = if dir == ExchangeDirection::BORBToExternal {
            ExchangeGateway::MobileMoney {
                provider: "MTN".to_string(),
                phone_number: "+243000000000".to_string(),
                country: "CD".to_string(),
            }
        } else {
            ExchangeGateway::UqInternal
        };

        let order = engine
            .create_order(&user, dir, source, destination, amount, 1.0)
            .map_err(|e| format!("{:?}", e))?;

        Ok(hex::encode(order.id))
    }

    /// Crée une offre de prêt
    pub fn create_lending_offer(
        &self,
        lender_hex: &str,
        amount: u64,
        min_cr: u64,
        max_days: u64,
        rate_bps: u64,
    ) -> Result<String, String> {
        let mut pool = self.lending.lock().unwrap();

        let lender_bytes = hex::decode(lender_hex).map_err(|_| "Adresse invalide")?;
        let mut lender = [0u8; 32];
        lender.copy_from_slice(&lender_bytes[..32]);

        let id = pool
            .create_offer(lender, amount, min_cr, max_days, rate_bps)
            .map_err(|e| format!("{:?}", e))?;

        Ok(hex::encode(id))
    }

    /// Emprunte
    pub fn borrow(
        &self,
        borrower_hex: &str,
        offer_id_hex: &str,
        amount: u64,
        collateral: u64,
        days: u64,
    ) -> Result<String, String> {
        let mut pool = self.lending.lock().unwrap();

        let borrower_bytes = hex::decode(borrower_hex).map_err(|_| "Adresse invalide")?;
        let mut borrower = [0u8; 32];
        borrower.copy_from_slice(&borrower_bytes[..32]);

        let offer_bytes = hex::decode(offer_id_hex).map_err(|_| "ID offre invalide")?;
        let mut offer_id = [0u8; 32];
        offer_id.copy_from_slice(&offer_bytes[..32]);

        let loan_id = pool
            .borrow(borrower, &offer_id, amount, collateral, days)
            .map_err(|e| format!("{:?}", e))?;

        Ok(hex::encode(loan_id))
    }

    /// Contribue à l'assurance
    pub fn insurance_contribute(&self, amount: u64) -> u64 {
        let mut fund = self.insurance.lock().unwrap();
        fund.contribute(amount)
    }

    /// Soumet une réclamation
    pub fn insurance_claim(
        &self,
        claimant_hex: &str,
        amount: u64,
        reason: &str,
    ) -> Result<String, String> {
        let mut fund = self.insurance.lock().unwrap();

        let claimant_bytes = hex::decode(claimant_hex).map_err(|_| "Adresse invalide")?;
        let mut claimant = [0u8; 32];
        claimant.copy_from_slice(&claimant_bytes[..32]);

        let trade_id = [0u8; 32];
        let evidence = vec![[0u8; 32]];

        let claim_id = fund
            .submit_claim(claimant, trade_id, amount, reason.to_string(), evidence)
            .map_err(|e| format!("{:?}", e))?;

        Ok(hex::encode(claim_id))
    }
}
