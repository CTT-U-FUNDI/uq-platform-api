use chrono::Utc;
use crate::robinet::Robinet;
use crate::escrow::Escrow;
use crate::session::PlayerSession;
use crate::payment::PaymentSplitter;
use crate::stats::PlayerStats;

/// Le lecteur UQ payant — le service KILLER
pub struct UqPlayer {
    pub session: Option<PlayerSession>,
    pub robinet: Robinet,
    pub escrow: Escrow,
    pub stats: PlayerStats,
}

impl UqPlayer {
    pub fn new() -> Self {
        Self {
            session: None,
            robinet: Robinet::new(),
            escrow: Escrow::new(),
            stats: PlayerStats::new(),
        }
    }
    
    /// Démarre une session de lecture
    pub fn start(&mut self, creator: [u8; 32], price_per_second: u64, max_duration_s: u64) -> Result<(), String> {
        let max_amount = price_per_second * max_duration_s;
        
        // Bloque le montant max en escrow
        self.escrow.deposit(max_amount)?;
        
        // Crée la session
        self.session = Some(PlayerSession::new(creator, price_per_second, Utc::now().timestamp()));
        self.robinet.start();
        self.stats.total_sessions += 1;
        
        Ok(())
    }
    
    /// Chaque seconde de lecture
    pub fn tick(&mut self) -> Result<(), String> {
        if let Some(session) = &mut self.session {
            if self.robinet.is_active() {
                session.seconds_consumed += 1;
                self.robinet.tick();
                self.stats.total_seconds += 1;
            }
        }
        Ok(())
    }
    
    /// Arrête la lecture
    pub fn stop(&mut self) -> Result<u64, String> {
        if let Some(session) = &self.session.take() {
            self.robinet.stop();
            
            let total_cost = session.seconds_consumed * session.price_per_second;
            let max_deposited = self.escrow.deposited_amount;
            let refund = max_deposited.saturating_sub(total_cost);
            
            // Rembourse le non-consommé
            self.escrow.refund(refund)?;
            
            // Paie le créateur (95%) et UQ (5%)
            let payment = PaymentSplitter::split(total_cost);
            self.escrow.pay(session.creator, payment.creator_amount)?;
            self.escrow.pay_to_six_cases(payment.uq_amount)?;
            
            self.stats.total_paid += total_cost;
            
            Ok(total_cost)
        } else {
            Err("Aucune session active".to_string())
        }
    }
}
