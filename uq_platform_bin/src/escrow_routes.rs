//! Escrow marketplace — flux achat/vente Zando.
//! Etats : interest -> discussion -> vente -> paiement -> confirme
use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn verrou_selon_prix(price_borb: f64) -> i64 {
    // Verrou proportionnel au prix : plus cher = plus de temps pour negocier
    if price_borb >= 1000.0 { 24 * 3600 }
    else if price_borb >= 100.0 { 6 * 3600 }
    else { 2 * 3600 }
}

fn db() -> Result<Connection, HttpResponse> {
    let c = Connection::open("uq_index.db").map_err(|_| HttpResponse::InternalServerError().finish())?;
    c.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS escrow_orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            product_id INTEGER NOT NULL,
            buyer TEXT NOT NULL,
            seller TEXT NOT NULL,
            price_borb REAL NOT NULL,
            status TEXT NOT NULL DEFAULT 'interest',
            temp_code TEXT,
            buyer_confirmed INTEGER DEFAULT 0,
            seller_confirmed INTEGER DEFAULT 0,
            buyer_lat REAL, buyer_lng REAL,
            seller_lat REAL, seller_lng REAL,
            locked_until INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS escrow_chat (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            order_id INTEGER NOT NULL,
            sender TEXT NOT NULL,
            msg TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_escrow_product ON escrow_orders(product_id, status);
        CREATE TABLE IF NOT EXISTS escrow_analytics (
            order_id INTEGER PRIMARY KEY,
            product_id INTEGER, buyer TEXT, seller TEXT,
            price_borb REAL, status TEXT,
            duree_secs INTEGER, cree_le INTEGER, clos_le INTEGER
        );
    "#).map_err(|_| HttpResponse::InternalServerError().finish())?;
    Ok(c)
}
fn now() -> i64 { chrono::Utc::now().timestamp() }

fn auth_uq(req: &HttpRequest) -> Option<String> {
    let auth = req.headers().get("Authorization")?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    crate::auth::AuthManager::verify_jwt(token).ok().map(|c| c.sub)
}

fn commande_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_,i64>(0)?, "product_id": r.get::<_,i64>(1)?,
        "buyer": r.get::<_,String>(2)?, "seller": r.get::<_,String>(3)?,
        "price_borb": r.get::<_,f64>(4)?, "status": r.get::<_,String>(5)?,
        "locked_until": r.get::<_,i64>(13)?,
    }))
}
const ORDER_COLS: &str = "id,product_id,buyer,seller,price_borb,status,locked_until";

// POST /api/escrow/buy — etape 1 : declaration d'intention
pub async fn escrow_buy(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let Some(buyer) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let product_id = body.get("product_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let Ok((seller, price)) = conn.query_row(
        "SELECT seller_uq, price_borb FROM marketplace_items WHERE id=?1 AND status='open'",
        params![product_id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,f64>(1)?))) else {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Produit indisponible"}));
    };
    if seller == buyer { return HttpResponse::BadRequest().json(json!({"status":"error","message":"Ton propre produit"})); }
    let exists: bool = conn.query_row(
        "SELECT COUNT(*)>0 FROM escrow_orders WHERE product_id=?1 AND status NOT IN ('confirme','annule')",
        params![product_id], |r| r.get(0)).unwrap_or(false);
    if exists { return HttpResponse::Conflict().json(json!({"status":"error","message":"Produit deja en discussion"})); }
    let t = now();
    let id = conn.execute(
        "INSERT INTO escrow_orders (product_id,buyer,seller,price_borb,status,locked_until,created_at,updated_at)
         VALUES (?1,?2,?3,?4,'interest',?5,?6,?6)",
        params![product_id,buyer,seller,price,t+verrou_selon_prix(price),t]).unwrap_or(0);
    HttpResponse::Ok().json(json!({"status":"success","order_id":id,"verrou_jusqua":t+verrou_selon_prix(price)}))
}

// GET /api/escrow/my-orders — commandes de l'user connecte
pub async fn escrow_my_orders(req: HttpRequest) -> HttpResponse {
    let Some(uq) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let mut st = conn.prepare(&format!(
        "SELECT {} FROM escrow_orders WHERE buyer=?1 OR seller=?1 ORDER BY updated_at DESC LIMIT 100", ORDER_COLS)).unwrap();
    let rows: Vec<_> = st.query_map(params![uq], |r| commande_json(r)).unwrap().flatten().collect();
    HttpResponse::Ok().json(json!({"status":"success","orders":rows}))
}

// GET /api/escrow/clients — etape 3 : vendeur voit produits + acheteurs interesses
pub async fn escrow_clients(req: HttpRequest) -> HttpResponse {
    let Some(seller) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let mut st = conn.prepare(
        "SELECT o.id, o.product_id, o.buyer, o.status, o.price_borb
         FROM escrow_orders o JOIN marketplace_items m ON m.id=o.product_id
         WHERE o.seller=?1 ORDER BY o.updated_at DESC").unwrap();
    let rows: Vec<_> = st.query_map(params![seller], |r| Ok(json!({
        "order_id": r.get::<_,i64>(0)?, "product_id": r.get::<_,i64>(1)?,
        "buyer": r.get::<_,String>(2)?, "status": r.get::<_,String>(3)?,
        "price_borb": r.get::<_,f64>(4)?
    }))).unwrap().flatten().collect();
    HttpResponse::Ok().json(json!({"status":"success","clients":rows}))
}

// POST /api/escrow/{id}/select — etape 4 : vendeur choisit -> discussion + verrou
pub async fn escrow_select(req: HttpRequest, path: web::Path<i64>, body: web::Json<Value>) -> HttpResponse {
    let id = path.into_inner();
    let Some(seller) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let buyer = body.get("buyer").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let n = conn.execute(
        "UPDATE escrow_orders SET status='discussion', buyer=?2, updated_at=?3
         WHERE id=?1 AND seller=?4 AND status='interest'",
        params![id, buyer, now(), seller]).unwrap_or(0);
    if n == 0 {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"Commande introuvable ou deja traitee"}));
    }
    if let Ok(pid) = conn.query_row("SELECT product_id FROM escrow_orders WHERE id=?1", params![id], |r| r.get::<_,i64>(0)) {
        let _ = conn.execute("UPDATE marketplace_items SET status='discussion' WHERE id=?1", params![pid]);
    }
    HttpResponse::Ok().json(json!({"status":"success","message":"Discussion ouverte — produit verrouille"}))
}

// POST /api/escrow/{id}/sell — etape 6 : vendeur "Vendre" -> vente en cours
pub async fn escrow_sell(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let id = path.into_inner();
    let Some(seller) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let n = conn.execute(
        "UPDATE escrow_orders SET status='vente', updated_at=?2 WHERE id=?1 AND seller=?3 AND status='discussion'",
        params![id, now(), seller]).unwrap_or(0);
    if n == 0 { return HttpResponse::BadRequest().json(json!({"status":"error","message":"Etat invalide"})); }
    HttpResponse::Ok().json(json!({"status":"success","message":"Vente en cours — suivi actif"}))
}

// POST /api/escrow/{id}/chat — message
pub async fn escrow_chat_send(req: HttpRequest, path: web::Path<i64>, body: web::Json<Value>) -> HttpResponse {
    let id = path.into_inner();
    let Some(uq) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let msg = body.get("msg").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if msg.is_empty() || msg.len() > 2000 { return HttpResponse::BadRequest().finish(); }
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    if !est_participant(&conn, id, &uq) { return HttpResponse::Forbidden().finish(); }
    conn.execute("INSERT INTO escrow_chat (order_id,sender,msg,created_at) VALUES (?1,?2,?3,?4)",
        params![id, uq, msg, now()]).ok();
    HttpResponse::Ok().json(json!({"status":"success"}))
}

// GET /api/escrow/{id}/chat — historique
pub async fn escrow_chat_get(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let id = path.into_inner();
    let Some(uq) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    if !est_participant(&conn, id, &uq) { return HttpResponse::Forbidden().finish(); }
    let mut st = conn.prepare("SELECT sender,msg,created_at FROM escrow_chat WHERE order_id=?1 ORDER BY id DESC LIMIT 100").unwrap();
    let msgs: Vec<_> = st.query_map(params![id], |r| Ok(json!({
        "sender": r.get::<_,String>(0)?, "msg": r.get::<_,String>(1)?, "at": r.get::<_,i64>(2)?
    }))).unwrap().flatten().collect();
    HttpResponse::Ok().json(json!({"status":"success","messages":msgs}))
}

fn est_participant(conn: &Connection, id: i64, uq: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*)>0 FROM escrow_orders WHERE id=?1 AND (buyer=?2 OR seller=?2)",
        params![id, uq], |r| r.get(0)).unwrap_or(false)
}

// POST /api/escrow/{id}/position — etape 7 : positions GPS temps reel
pub async fn escrow_position(req: HttpRequest, path: web::Path<i64>, body: web::Json<Value>) -> HttpResponse {
    let id = path.into_inner();
    let Some(uq) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let lat = body.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lng = body.get("lng").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if lat == 0.0 && lng == 0.0 { return HttpResponse::BadRequest().json(json!({"status":"error","message":"Position requise"})); }
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let role: Option<String> = conn.query_row(
        "SELECT CASE WHEN buyer=?2 THEN 'buyer' WHEN seller=?2 THEN 'seller' END FROM escrow_orders WHERE id=?1",
        params![id, uq], |r| r.get(0)).ok();
    let Some(role) = role else { return HttpResponse::Forbidden().finish() };
    let (lat_col, lng_col) = if role == "buyer" { ("buyer_lat", "buyer_lng") } else { ("seller_lat", "seller_lng") };
    conn.execute(&format!("UPDATE escrow_orders SET {}=?2, {}=?3, updated_at=?4 WHERE id=?1", lat_col, lng_col),
        params![id, lat, lng, now()]).ok();
    HttpResponse::Ok().json(json!({"status":"success"}))
}

// POST /api/escrow/{id}/confirm — etape 10 : double confirmation + code croise 4 chiffres
pub async fn escrow_confirm(req: HttpRequest, path: web::Path<i64>, body: web::Json<Value>) -> HttpResponse {
    let id = path.into_inner();
    let Some(uq) = auth_uq(&req) else { return HttpResponse::Unauthorized().finish() };
    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let conn = match db() { Ok(c)=>c, Err(r)=>return r };
    let row = conn.query_row(
        "SELECT buyer,seller,status,temp_code,price_borb FROM escrow_orders WHERE id=?1",
        params![id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?,
                            r.get::<_,Option<String>>(3)?, r.get::<_,f64>(4)?)));
    let Ok((buyer, seller, status, temp, price)) = row else { return HttpResponse::NotFound().finish() };
    if status != "vente" && status != "paiement" {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"Pas en attente de paiement"}));
    }
    let is_buyer = uq == buyer;
    let is_seller = uq == seller;
    if !is_buyer && !is_seller { return HttpResponse::Forbidden().finish(); }

    // Premier "paiement reussi" -> genere le code 4 chiffres (preuve des 2 confirmations)
    if temp.is_none() {
        let c4 = format!("{:04}", rand_code());
        conn.execute("UPDATE escrow_orders SET temp_code=?2, status='paiement', updated_at=?3 WHERE id=?1",
            params![id, c4, now()]).ok();
        return HttpResponse::Ok().json(json!({"status":"success","temp_code":c4,"message":"Code temporaire genere — montre-le a l'autre partie"}));
    }
    // Verification croisee : le code saisi doit correspondre
    if code != temp.clone().unwrap_or_default() {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"Code incorrect — litige"}));
    }
    let col = if is_buyer { "buyer_confirmed" } else { "seller_confirmed" };
    conn.execute(&format!("UPDATE escrow_orders SET {}=1, updated_at=?2 WHERE id=?1", col), params![id, now()]).ok();
    let (bc, sc): (i64, i64) = conn.query_row(
        "SELECT buyer_confirmed, seller_confirmed FROM escrow_orders WHERE id=?1", params![id],
        |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((0, 0));
    if bc == 1 && sc == 1 {
        conn.execute("UPDATE escrow_orders SET status='confirme', updated_at=?2 WHERE id=?1", params![id, now()]).ok();
        // le produit disparait du flux Zando une fois vendu
        let _ = conn.execute("UPDATE marketplace_items SET status='sold' WHERE id=?1", params![conn.query_row("SELECT product_id FROM escrow_orders WHERE id=?2",params![id],|r|r.get::<_,i64>(0)).unwrap_or(0)]);
        // PURGE post-evenement : historique chat efface, seule synthese analytics conservee
        purge_et_analytics(&conn, id);
        liberer_escrow(&seller, &buyer, price, id);
        return HttpResponse::Ok().json(json!({"status":"success","message":"VENTE CONFIRMEE — escrow libere","sold":true}));
    }
    HttpResponse::Ok().json(json!({"status":"success","message":"Confirmation enregistree — en attente de l'autre partie"}))
}

fn rand_code() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos() as u64;
    ((seed ^ (seed >> 16)) % 10000) as u32
}

fn liberer_escrow(seller: &str, buyer: &str, montant: f64, commande_id: i64) {
    // Escrow BORB : vend les points geles -> acheteur (points = BORB x 100)
    let points = (montant * 100.0).round() as i64;
    if points <= 0 { return; }
    let t = now();
    if crate::p2p_escrow::liberer(seller, buyer, points, commande_id, t).is_err() {
        eprintln!("🚨 ESCROW liberer echoue {} -> {} ({} pts)", seller, buyer, points);
    }
}

/// Purge le chat d'une commande cloturee et garde une fiche analytics minimale.
fn purge_et_analytics(conn: &Connection, id: i64) {
    let Ok((pid, buyer, seller, price, created)) = conn.query_row(
        "SELECT product_id,buyer,seller,price_borb,created_at FROM escrow_orders WHERE id=?1",
        params![id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
                            r.get::<_,f64>(3)?,r.get::<_,i64>(4)?))) else { return };
    let _ = conn.execute("DELETE FROM escrow_chat WHERE order_id=?1", params![id]);
    let duree = (now() - created).max(0) as i64;
    let _ = conn.execute(
        "INSERT OR REPLACE INTO escrow_analytics (order_id,product_id,buyer,seller,price_borb,status,duree_secs,cree_le,clos_le)
         VALUES (?1,?2,?3,?4,?5,'confirme',?6,?7,?8)",
        params![id,pid,buyer,seller,price,duree,created,now()]);
}

/// Purge aussi les commandes annulees/expirees (nettoyage periodique).
pub fn purge_cloturees() {
    if let Ok(conn) = db() {
        // commandes non actives depuis +24h -> purge chat + analytics
        let vieux = now() - 86400;
        let ids: Vec<i64> = conn.prepare(
            "SELECT id FROM escrow_orders WHERE status IN ('confirme','annule') AND updated_at<?1")
            .and_then(|mut s| s.query_map(params![vieux], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        for id in ids { purge_et_analytics(&conn, id); }
    }
}
