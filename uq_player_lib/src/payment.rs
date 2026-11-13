/// La répartition des paiements — 95% créateur / 5% UQ
pub struct PaymentSplit {
    pub creator_amount: u64,
    pub uq_amount: u64,
}

pub struct PaymentSplitter;

impl PaymentSplitter {
    pub fn split(total: u64) -> PaymentSplit {
        let creator = total * 95 / 100;
        let uq = total - creator;
        
        PaymentSplit {
            creator_amount: creator,
            uq_amount: uq,
        }
    }
}
