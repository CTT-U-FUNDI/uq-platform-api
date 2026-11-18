// ====================
// PERMISSIONS — Calculées en temps réel
// ====================

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    pub is_owner: bool,
    pub kyc_verified: bool,
    pub cadenas_verrouille: bool,
    pub pin_enabled: bool,
    pub coffre_access: bool,
    pub wallet_access: bool,
    pub borb_operations: bool,
    pub ubi_claim: bool,
    pub zando_publish: bool,
    pub can_send: bool,
    pub can_receive: bool,
    pub can_convert: bool,
    pub can_ubi: bool,
    pub can_publish: bool,
    pub can_trade: bool,
    pub can_save: bool,
    pub can_borrow: bool,
    pub can_access_treasury: bool,
    pub can_access_cases: bool,
    pub can_access_fraud: bool,
    pub can_arbitrate: bool,
    pub can_validate: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Permissions {
            is_owner: false,
            kyc_verified: false,
            cadenas_verrouille: true,
            pin_enabled: false,
            coffre_access: false,
            wallet_access: false,
            borb_operations: false,
            ubi_claim: false,
            zando_publish: false,
            can_send: false,
            can_receive: true,
            can_convert: true,
            can_ubi: true,
            can_publish: true,
            can_trade: false,
            can_save: true,
            can_borrow: false,
            can_access_treasury: false,
            can_access_cases: false,
            can_access_fraud: false,
            can_arbitrate: false,
            can_validate: false,
        }
    }
}

pub fn compute_permissions(uq_number: &str) -> Permissions {
    let mut perms = Permissions::default();

    // 1. uq_index (is_owner)
    let idx = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path());
    if let Some(account) = idx.get_account(uq_number) {
        perms.is_owner = account.get("is_owner").and_then(|v| v.as_bool()).unwrap_or(false);
    }

    // 2. DB du compte
    let db_path = format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase());
    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(_) => {
            perms.can_access_treasury = perms.is_owner;
            perms.can_access_cases = perms.is_owner;
            perms.can_access_fraud = perms.is_owner;
            perms.can_arbitrate = perms.is_owner;
            perms.can_validate = perms.is_owner;
            return perms;
        }
    };

    // Cadenas KYC
    if let Ok((verrouille, est_verifie)) = conn.query_row::<(i64, i64), _, _>(
        "SELECT verrouille, est_verifie FROM cadenas_verification WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        perms.cadenas_verrouille = verrouille == 1;
        perms.kyc_verified = est_verifie == 1 && verrouille == 0;
    }

    // PIN
    if let Ok(enabled) = conn.query_row::<i64, _, _>(
        "SELECT pin_enabled FROM pin_status WHERE id = 1",
        [],
        |row| row.get(0),
    ) {
        perms.pin_enabled = enabled == 1;
    }

    // Privilèges
    let privileges: Vec<(String, i64)> = conn
        .prepare("SELECT privilege_name, enabled FROM privileges")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            Ok(rows.flatten().collect())
        })
        .unwrap_or_default();

    for (name, enabled) in &privileges {
        if *enabled != 1 { continue; }
        match name.as_str() {
            "coffre_access" => perms.coffre_access = true,
            "wallet_access" => perms.wallet_access = true,
            "borb_operations" => perms.borb_operations = true,
            "ubi_claim" => perms.ubi_claim = true,
            "zando_publish" => perms.zando_publish = true,
            _ => {}
        }
    }

    // Calculs
    perms.can_send = perms.kyc_verified || perms.is_owner;
    perms.can_trade = perms.kyc_verified || perms.is_owner;
    perms.can_borrow = perms.kyc_verified || perms.is_owner;
    perms.can_access_treasury = perms.is_owner;
    perms.can_access_cases = perms.is_owner;
    perms.can_access_fraud = perms.is_owner;
    perms.can_arbitrate = perms.is_owner;
    perms.can_validate = perms.is_owner;

    perms
}

pub fn permissions_to_json(perms: &Permissions) -> Value {
    json!({
        "is_owner": perms.is_owner,
        "kyc": {
            "verified": perms.kyc_verified,
            "cadenas_verrouille": perms.cadenas_verrouille
        },
        "pin": { "enabled": perms.pin_enabled },
        "permissions": {
            "can_send": perms.can_send,
            "can_receive": perms.can_receive,
            "can_convert": perms.can_convert,
            "can_ubi": perms.can_ubi,
            "can_publish": perms.can_publish,
            "can_trade": perms.can_trade,
            "can_save": perms.can_save,
            "can_borrow": perms.can_borrow,
            "can_access_treasury": perms.can_access_treasury,
            "can_access_cases": perms.can_access_cases,
            "can_access_fraud": perms.can_access_fraud,
            "can_arbitrate": perms.can_arbitrate,
            "can_validate": perms.can_validate
        }
    })
}