use crate::models::{DataOffer, MarketStats};


pub struct DataMarket {
    pub offers: Vec<DataOffer>,
    pub total_data_sold_mb: u64,
    pub total_volume_micro: u64,
}

pub fn new() -> DataMarket {
    DataMarket { offers: Vec::new(), total_data_sold_mb: 0, total_volume_micro: 0 }
}

pub fn register_offer(market: &mut DataMarket, offer: DataOffer) {
    market.offers.push(offer);
}

pub fn get_offers(market: &DataMarket) -> &[DataOffer] {
    &market.offers
}

pub fn get_stats(market: &DataMarket) -> MarketStats {
    let active = market.offers.iter().filter(|o| o.active).count() as u64;
    let avg_price = if active > 0 { market.offers.iter().map(|o| o.price_per_mb_micro).sum::<u64>() / active } else { 0 };
    MarketStats {
        active_sellers: active, active_buyers: 0,
        total_data_sold_mb: market.total_data_sold_mb,
        total_volume_micro: market.total_volume_micro,
        average_price_per_mb: avg_price,
    }
}



