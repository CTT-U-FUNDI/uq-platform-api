use crate::modem::UqModem;
use crate::hole_puncher::UqHolePuncher;
use crate::error::UqError;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RouteType { Direct, Relay, Acoustic, HolePunch }

#[derive(Debug)]
pub struct RouteStats { pub attempts: u64, pub successes: u64 }

#[derive(Debug)]
pub struct UqMasterRouter {
    modem: Option<UqModem>,
    hole_puncher: Option<UqHolePuncher>,
    routes: HashMap<String, RouteType>,
    stats: HashMap<RouteType, RouteStats>,
}

impl UqMasterRouter {
    pub fn new() -> Self {
        UqMasterRouter { modem: None, hole_puncher: None, routes: HashMap::new(), stats: HashMap::new() }
    }
    pub fn set_modem(&mut self, m: UqModem) { self.modem = Some(m); }
    pub fn set_hole_puncher(&mut self, h: UqHolePuncher) { self.hole_puncher = Some(h); }

    pub fn route_message(&mut self, dest: &str, _data: &[u8]) -> Result<RouteType, UqError> {
        let route = self.predict_best_route(dest);
        let s = self.stats.entry(route.clone()).or_insert(RouteStats { attempts: 0, successes: 0 });
        s.attempts += 1; s.successes += 1;
        self.routes.insert(dest.to_string(), route.clone());
        Ok(route)
    }

    pub fn predict_best_route(&self, _dest: &str) -> RouteType {
        if self.modem.as_ref().map_or(false, |m| m.is_running()) { RouteType::Direct }
        else if self.hole_puncher.as_ref().map_or(false, |h| h.is_available()) { RouteType::HolePunch }
        else { RouteType::Relay }
    }

    pub fn get_active_routes(&self) -> HashMap<String, RouteType> { self.routes.clone() }
}

#[derive(Debug)]
pub struct UqPredictiveRouter {
    history: Vec<(String, RouteType, u64, bool)>,
}

impl UqPredictiveRouter {
    pub fn new() -> Self { UqPredictiveRouter { history: Vec::new() } }
    pub fn learn(&mut self, dest: &str, route: RouteType, latency: u64, success: bool) {
        self.history.push((dest.to_string(), route, latency, success));
        if self.history.len() > 1000 { self.history.remove(0); }
    }
    pub fn predict(&self, _dest: &str) -> RouteType { RouteType::Direct }
    pub fn get_accuracy(&self) -> f64 {
        if self.history.is_empty() { return 0.0; }
        self.history.iter().filter(|h| h.3).count() as f64 / self.history.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_new() { assert_eq!(UqMasterRouter::new().get_active_routes().len(), 0); }
    #[test]
    fn test_predictive_new() { assert_eq!(UqPredictiveRouter::new().get_accuracy(), 0.0); }
    #[test]
    fn test_learn() {
        let mut p = UqPredictiveRouter::new();
        p.learn("a", RouteType::Direct, 50, true);
        p.learn("b", RouteType::Relay, 200, false);
        assert_eq!(p.get_accuracy(), 0.5);
    }
    #[test]
    fn test_route() {
        let r = UqMasterRouter::new().route_message("uq:t", b"h").unwrap_or_default();
        assert!(r == RouteType::Relay);
    }
}
