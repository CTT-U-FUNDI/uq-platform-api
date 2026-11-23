// ====================
// 🚌 BUS — Orchestration des moteurs et du gossip
// ====================
//
// Chaque moteur LOCAL publie via publier() (version monotone par
// origine) ; les pairs envoient leurs Diffusion vers recevoir(), qui
// vérifie la signature, déduplique, fusionne, et re-diffuse jusqu'à
// MAX_SAUT. Le moteur « synchro » ne sort jamais vers le mesh ouvert :
// il ne vise que les appareils appairés du même compte.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use sha2::Digest as _;

use super::gossip::{self, Diffusion};
use super::store::StoreMoteurs;
use crate::udp_network::UdpNetwork;

pub const MOTEUR_ORACLE: &str = "oracle";
pub const MOTEUR_FRAUDE: &str = "fraude";
pub const MOTEUR_REPUTATION: &str = "reputation";
pub const MOTEUR_SYNCHRO: &str = "synchro";

fn env_positif(nom: &str, defaut: u64) -> u64 {
    std::env::var(nom)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &u64| *v > 0)
        .unwrap_or(defaut)
}

fn max_saut() -> u8 {
    env_positif("UQ_ENGINE_MAX_SAUT", 4).min(255) as u8
}

pub struct BusMoteurs {
    store: Mutex<StoreMoteurs>,
    udp: Arc<UdpNetwork>,
    origine: String,
    pub diffusees: AtomicU64,
    pub recues: AtomicU64,
    pub rejetees: AtomicU64,
}

impl BusMoteurs {
    pub fn nouvelles(udp: Arc<UdpNetwork>) -> Arc<Self> {
        let origine = gossip::identifiant_appareil();
        Arc::new(Self {
            store: Mutex::new(StoreMoteurs::ouvrir()),
            udp,
            origine: origine[..16].to_string(),
            diffusees: AtomicU64::new(0),
            recues: AtomicU64::new(0),
            rejetees: AtomicU64::new(0),
        })
    }

    fn serialiser(d: &Diffusion) -> String {
        format!(
            "{}{}",
            gossip::PREFIXE_GOSSIP,
            serde_json::to_string(d).unwrap_or_default()
        )
    }

    fn diffuser_udp(&self, d: &Diffusion) -> usize {
        let message = Self::serialiser(d);
        if d.moteur == MOTEUR_SYNCHRO {
            // Jamais de flood ouvert pour les données métier : uniquement
            // les adresses appairées du compte destinataire.
            let adresses: Vec<String> = self
                .store
                .lock()
                .map(|s| s.appareils_de(&d.destinataire.clone().unwrap_or_default()))
                .unwrap_or_default();
            let mut n = 0;
            for addr in adresses {
                if self.udp.send(&addr, &message).is_ok() {
                    n += 1;
                }
            }
            // l'appareil émetteur lui-même est joint via HTTP, pas UDP
            n
        } else {
            self.udp.broadcast(&message)
        }
    }

    /// Publication d'une mise à jour locale : version suivante pour
    /// (moteur, cle, nous), stockage, puis diffusion aux pairs.
    pub fn publier(
        &self,
        moteur: &str,
        cle: &str,
        valeur: serde_json::Value,
        destinataire: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let origine = self.origine.clone();
        let version = {
            let store = self.store.lock().map_err(|_| "lock store")?;
            store.version_origine(moteur, cle, &origine) + 1
        };
        let mut d = Diffusion {
            moteur: moteur.to_string(),
            cle: cle.to_string(),
            version,
            valeur,
            ts: gossip::maintenant(),
            origine,
            destinataire: destinataire.map(|s| s.to_string()),
            saut: 0,
            signature: String::new(),
        };
        gossip::signer(&mut d);
        {
            let mut store = self.store.lock().map_err(|_| "lock store")?;
            store.appliquer(&d)?;
        }
        let vises = self.diffuser_udp(&d);
        self.diffusees.fetch_add(1, Ordering::Relaxed);
        Ok(json!({
            "status": "success",
            "moteur": d.moteur,
            "cle": d.cle,
            "version": d.version,
            "empreinte": d.empreinte(),
            "pairs_vises": vises,
        }))
    }

    /// Réception d'une diffusion (UDP ou HTTP). Vérifie, déduplique,
    /// fusionne, et re-diffuse si elle est nouvelle.
    pub fn recevoir(
        &self,
        mut d: Diffusion,
        depuis: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.recues.fetch_add(1, Ordering::Relaxed);
        if !gossip::signature_valide(&d) {
            self.rejetees.fetch_add(1, Ordering::Relaxed);
            return Err("Signature invalide ou absente".to_string());
        }
        let hash = d.empreinte();
        let deja_vu = self.store.lock().map(|s| s.deja_vu(&hash)).unwrap_or(false);
        let retenue = {
            let mut store = self.store.lock().map_err(|_| "lock store")?;
            store.appliquer(&d)?
        };
        if !retenue {
            return Ok(
                json!({"status":"success","accepte":false,"motif": if deja_vu {"deja_vu"} else {"version_plus_ancienne"}}),
            );
        }

        // Le moteur fraude surveille l'oracle : un prix pair très écarté
        // de la médiane collective est signalé, sans être censuré.
        if d.moteur == MOTEUR_ORACLE {
            surveiller_ecart(self, &d);
        }

        if d.saut < max_saut() {
            d.saut += 1;
            self.diffuser_udp(&d);
        }
        let _ = depuis;
        Ok(
            json!({"status":"success","accepte":true,"moteur":d.moteur,"cle":d.cle,"version":d.version,"empreinte":hash}),
        )
    }

    /// Agrégat oracle : pour chaque clé, médiane + dispersion sur toutes
    /// les origines, avec la valeur locale côte à côte.
    pub fn agregat_oracle(&self) -> serde_json::Value {
        let store = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return json!({"status":"error"}),
        };
        let lignes = store.lignes(MOTEUR_ORACLE);
        let mut cles: HashSet<String> = HashSet::new();
        for (cle, _, _, _) in &lignes {
            cles.insert(cle.clone());
        }
        let maintenant = gossip::maintenant();
        let entrees: Vec<serde_json::Value> = cles
            .into_iter()
            .map(|cle| {
                let (mediane, origines) =
                    store.mediane_prix(MOTEUR_ORACLE, &cle).unwrap_or((0.0, 0));
                let locale = lignes
                    .iter()
                    .find(|(c, o, _, _)| *c == cle && *o == self.origine)
                    .and_then(|(_, _, v, _)| v.get("prix").and_then(|p| p.as_f64()));
                let ages: Vec<i64> = lignes
                    .iter()
                    .filter(|(c, _, _, _)| *c == cle)
                    .map(|(_, _, _, ts)| maintenant - *ts)
                    .collect();
                json!({
                    "cle": cle,
                    "mediane": mediane,
                    "origines": origines,
                    "local": locale,
                    "age_max_sec": ages.iter().max().copied().unwrap_or(-1),
                })
            })
            .collect();
        json!({"status":"success","mediane_collective": entrees, "min_origines_mediane": min_origines_mediane()})
    }

    /// Vue collective d'un compte : somme des attestations de toutes origines.
    pub fn agregat_reputation(&self, uq: &str) -> serde_json::Value {
        let cle = format!("compte:{}", uq);
        let store = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return json!({"status":"error"}),
        };
        let lignes: Vec<_> = store
            .lignes(MOTEUR_REPUTATION)
            .into_iter()
            .filter(|(c, _, _, _)| *c == cle)
            .collect();
        let total_tx: i64 = lignes
            .iter()
            .map(|(_, _, v, _)| v.get("nb_tx").and_then(|n| n.as_i64()).unwrap_or(0))
            .sum();
        let volume: i64 = lignes
            .iter()
            .map(|(_, _, v, _)| v.get("volume_points").and_then(|n| n.as_i64()).unwrap_or(0))
            .sum();
        let dernier = lignes
            .iter()
            .filter_map(|(_, _, v, _)| v.get("dernier_ts").and_then(|n| n.as_i64()))
            .max()
            .unwrap_or(0);
        json!({
            "status":"success","uq":uq,"cle":cle,
            "attestations": lignes.len(), "total_tx": total_tx,
            "volume_points": volume, "dernier_ts": dernier,
            "detail": lignes.iter().map(|(_,o,v,ts)| json!({"origine":o,"valeur":v,"ts":ts})).collect::<Vec<_>>(),
        })
    }

    /// Events synchro stockés pour un compte (provenant de ses appareils).
    pub fn agregat_synchro(&self, uq: &str) -> serde_json::Value {
        let store = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return json!({"status":"error"}),
        };
        let prefixe = format!("event:{}", uq);
        let mut events: Vec<_> = store
            .lignes(MOTEUR_SYNCHRO)
            .into_iter()
            .filter(|(c, _, _, _)| c.starts_with(&prefixe))
            .map(|(c, o, v, ts)| json!({"cle":c,"origine":o,"evenement":v,"ts":ts}))
            .collect();
        events.sort_by(|a, b| {
            b["ts"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["ts"].as_i64().unwrap_or(0))
        });
        events.truncate(200);
        json!({"status":"success","uq":uq,"evenements":events,"total":events.len()})
    }

    pub fn etat(&self) -> serde_json::Value {
        let store = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return json!({"status":"error","message":"store occupé"}),
        };
        let par_moteur = ["oracle", "fraude", "reputation", "synchro"]
            .iter()
            .map(|m| {
                let lignes = store.lignes(m);
                let mut cles: HashSet<&String> = HashSet::new();
                for (c, _, _, _) in &lignes {
                    cles.insert(c);
                }
                json!({"cles": cles.len(), "origines": lignes.len()})
            })
            .collect::<serde_json::Value>();
        json!({
            "status": "success",
            "appareil": self.origine,
            "pairs_udp": self.udp.peer_count(),
            "compteurs": {
                "diffusees": self.diffusees.load(Ordering::Relaxed),
                "recues": self.recues.load(Ordering::Relaxed),
                "rejetees": self.rejetees.load(Ordering::Relaxed),
            },
            "moteurs": par_moteur,
            "store": store.stats(),
        })
    }

    pub fn pairs(&self) -> serde_json::Value {
        let store = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return json!({"status":"error"}),
        };
        json!({
            "status":"success",
            "udp": self.udp.get_peers(),
            "appareils": store.liste_appareils().iter().map(|(uq,ap,addr,ts)| json!({"uq":uq,"appareil":ap,"addr":addr,"appaire":ts})).collect::<Vec<_>>(),
        })
    }

    pub fn appairer(&self, uq: &str, appareil: &str, addr: &str) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|_| "lock store")?;
        store.appairer(uq, appareil, addr, gossip::maintenant())
    }

    /// Déclare une adresse comme pair UDP et lui envoie la poignée de main.
    pub fn notifier_pair(&self, addr: &str) {
        self.udp.add_peer(addr);
        let _ = self.udp.send(addr, "UQ_HANDSHAKE");
    }

    /// Le pump UDP du bus : consomme le canal de réception de UdpNetwork.
    pub fn brancher_reception_udp(bus: Arc<BusMoteurs>) {
        std::thread::spawn(move || {
            let Some(rx) = bus.udp.prendre_reception() else {
                eprintln!("⚠️  Réception UDP déjà consommée — moteurs sans pump UDP");
                return;
            };
            for (addr, message) in rx.iter() {
                let Some(corps) = message.strip_prefix(gossip::PREFIXE_GOSSIP) else {
                    continue;
                };
                match serde_json::from_str::<Diffusion>(corps) {
                    Ok(d) => {
                        if let Err(e) = bus.recevoir(d, Some(&addr)) {
                            eprintln!("🧠 Gossip de {} refusé : {}", addr, e);
                        }
                    }
                    Err(e) => eprintln!("🧠 Gossip de {} illisible : {}", addr, e),
                }
            }
        });
    }
}

fn min_origines_mediane() -> u64 {
    env_positif("UQ_ENGINE_MIN_ORIGINES", 3)
}

fn ecart_max() -> f64 {
    std::env::var("UQ_ORACLE_ECART_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15.0)
}

/// Signale en moteur « fraude » un prix pair qui s'écarte trop de la
/// médiane collective (au moins `min_origines` sources pour juger).
fn surveiller_ecart(bus: &BusMoteurs, d: &Diffusion) {
    let Some(prix) = d.valeur.get("prix").and_then(|p| p.as_f64()) else {
        return;
    };
    let Some((mediane, origines)) = bus
        .store
        .lock()
        .ok()
        .and_then(|s| s.mediane_prix(MOTEUR_ORACLE, &d.cle))
    else {
        return;
    };
    if (origines as u64) < min_origines_mediane() || mediane <= 0.0 {
        return;
    }
    let ecart_pct = ((prix - mediane).abs() / mediane) * 100.0;
    if ecart_pct <= ecart_max() {
        return;
    }
    let jour = d.ts / 86400;
    let cle = format!("oracle-ecart:{}:{}:{}", d.cle, d.origine, jour);
    if bus
        .store
        .lock()
        .map(|s| s.version_origine(MOTEUR_FRAUDE, &cle, &bus.origine) > 0)
        .unwrap_or(true)
    {
        return;
    }
    let _ = bus.publier(
        MOTEUR_FRAUDE,
        &cle,
        json!({
            "type": "oracle_ecart",
            "cle": d.cle,
            "accuse": d.origine,
            "prix_signale": prix,
            "mediane": mediane,
            "ecart_pct": ecart_pct,
            "constate_par": bus.origine,
        }),
        None,
    );
}

// ====================
// BOUCLES DES MOTEURS — sources locales RÉELLES
// ====================

/// Oracle : taux fiat du jour + prix BORB interne + crypto si configuré.
pub async fn tick_oracle(bus: Arc<BusMoteurs>) {
    let taux = crate::public_routes::taux_du_jour().await;
    for (devise, valeur) in &taux.rates {
        let _ = bus.publier(
            MOTEUR_ORACLE,
            &format!("fx:{}", devise),
            json!({"prix": valeur, "base": taux.base, "source": taux.source}),
            None,
        );
    }
    let prix = crate::persistence::load_prix();
    let _ = bus.publier(
        MOTEUR_ORACLE,
        "borb",
        json!({"prix": prix.prix_actuel, "demande": prix.demande, "rarete": prix.rarete}),
        None,
    );
    // Crypto : symboles explicites via UQ_ENGINE_CRYPTO_SYMS (ex. « BTCUSDT,ETHUSDT »)
    let symboles: Vec<String> = std::env::var("UQ_ENGINE_CRYPTO_SYMS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    for symbole in symboles {
        let recu = crate::external_api::prix_crypto(&symbole).await;
        if let Some(prix) = recu
            .get("data")
            .and_then(|d| d.get("price"))
            .and_then(|p| p.as_f64())
        {
            let source = recu
                .get("source")
                .and_then(|s| s.as_str())
                .unwrap_or("inconnue");
            let _ = bus.publier(
                MOTEUR_ORACLE,
                &format!("crypto:{}", symbole),
                json!({"prix": prix, "source": source}),
                None,
            );
        }
    }
}

/// Réputation : attestation locale sur chaque compte vu dans les
/// transactions observées par CETTE instance (nb, volume, dernier).
pub fn tick_reputation(bus: &BusMoteurs) {
    let db = crate::database::Database::shared("uq_platform_db.json");
    let comptes = db.get_transaction_comptes();
    let capacite = env_positif("UQ_ENGINE_REPUTATION_MAX", 200) as usize;
    for uq in comptes.into_iter().take(capacite) {
        let txs = db.get_transactions_compte(&uq, usize::MAX);
        if txs.is_empty() {
            continue;
        }
        let volume: u64 = txs.iter().map(|t| t.amount_points).sum();
        let dernier_ts = txs.iter().map(|t| t.timestamp).max().unwrap_or(0);
        let _ = bus.publier(
            MOTEUR_REPUTATION,
            &format!("compte:{}", uq),
            json!({"nb_tx": txs.len(), "volume_points": volume, "dernier_ts": dernier_ts}),
            None,
        );
    }
}

/// Fraude : les sanctions enregistrées localement deviennent des
/// signalements collectifs (clé = empreinte, diffusion une seule fois).
pub fn tick_fraude(bus: &BusMoteurs) {
    let db = crate::database::Database::shared("uq_platform_db.json");
    let origine = bus.origine.clone();
    for sanction in db.get_sanctions(1000) {
        let cible = sanction
            .get("account_id")
            .or_else(|| sanction.get("uq"))
            .and_then(|v| v.as_str())
            .unwrap_or("inconnu")
            .to_string();
        let seriel = serde_json::to_string(&sanction).unwrap_or_default();
        let mut hasher = sha2::Sha256::new();
        sha2::Digest::update(&mut hasher, seriel.as_bytes());
        let empreinte = hex::encode(sha2::Digest::finalize(hasher));
        let cle = format!("sanction:{}:{}", cible, &empreinte[..12]);
        let deja = bus
            .store
            .lock()
            .map(|s| s.version_origine(MOTEUR_FRAUDE, &cle, &origine) > 0)
            .unwrap_or(true);
        if deja {
            continue;
        }
        let _ = bus.publier(
            MOTEUR_FRAUDE,
            &cle,
            json!({"type": "sanction", "cible": cible, "detail": sanction}),
            None,
        );
    }
}

/// Boucle générale : chaque moteur a son rythme (env, défauts ci-dessous).
pub async fn boucle_moteurs(bus: Arc<BusMoteurs>) {
    use tokio::time::{interval, Duration};
    let tick_oracle_s = env_positif("UQ_ENGINE_TICK_ORACLE_SECS", 60);
    let tick_rep_s = env_positif("UQ_ENGINE_TICK_REPUTATION_SECS", 900);
    let tick_fraude_s = env_positif("UQ_ENGINE_TICK_FRAUDE_SECS", 300);

    let mut ticker_oracle = interval(Duration::from_secs(tick_oracle_s));
    let mut ticker_rep = interval(Duration::from_secs(tick_rep_s));
    let mut ticker_fraude = interval(Duration::from_secs(tick_fraude_s));
    ticker_oracle.tick().await;
    loop {
        tokio::select! {
            _ = ticker_oracle.tick() => {
                tick_oracle(Arc::clone(&bus)).await;
            }
            _ = ticker_rep.tick() => {
                tick_reputation(&bus);
            }
            _ = ticker_fraude.tick() => {
                tick_fraude(&bus);
            }
        }
    }
}
