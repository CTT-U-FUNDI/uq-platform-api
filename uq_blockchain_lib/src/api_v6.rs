use serde::{Deserialize, Serialize};
use crate::bubble::BubbleRegistry;
use crate::six_cases::SixCases;
use crate::error::ChainError;

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
    pub timestamp: u64,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        ApiResponse { success: true, data: Some(data), error: None, timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }
    }
    pub fn err(msg: &str) -> Self {
        ApiResponse { success: false, data: None, error: Some(msg.to_string()), timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceResponse { pub balance_BORB: u64, pub balance_micro: u64 }

#[derive(Debug, Serialize)]
pub struct PlatformStats { pub total_transactions: u64, pub total_users: u64, pub tx_count: u64 }

#[derive(Debug, Deserialize)]
pub struct RegisterUserRequest { pub email: String, pub name: String, pub phone: String, pub country: String }

#[derive(Debug, Serialize)]
pub struct RegisterUserResponse { pub success: bool, pub uq_number: String, pub email_uq: String }

#[derive(Debug, Deserialize)]
pub struct RecoverUserRequest { pub uq_number: String, pub fragments: Vec<String> }

#[derive(Debug, Serialize)]
pub struct RecoverUserResponse { pub success: bool, pub new_uq_number: String }

#[derive(Debug, Deserialize)]
pub struct VerifyOtpRequest { pub phone: String, pub code: String }

#[derive(Debug, Serialize)]
pub struct VerifyOtpResponse { pub success: bool, pub verified: bool }

pub struct UqApi { pub six_cases: SixCases, pub registry: BubbleRegistry }

impl UqApi {
    pub fn new(six_cases: SixCases, registry: BubbleRegistry) -> Self { UqApi { six_cases, registry } }

    pub fn get_balance(&self, address: &[u8; 32]) -> ApiResponse<BalanceResponse> {
        let balance = self.registry.get_balance(address);
        ApiResponse::ok(BalanceResponse { balance_BORB: balance / 1_000_000, balance_micro: balance })
    }

    pub fn collect_fee(&mut self, amount: u64) -> Result<ApiResponse<u64>, ChainError> {
        let fee = self.six_cases.collect_fee(amount)?;
        Ok(ApiResponse::ok(fee))
    }

    /// Retourne les stats des cases (CaseStats a SEULEMENT name et balance)
    pub fn get_cases_stats(&self) -> ApiResponse<Vec<(String, u64)>> {
        let stats = self.six_cases.get_stats();
        let result: Vec<(String, u64)> = stats.iter().map(|s| (s.name.clone(), s.balance)).collect();
        ApiResponse::ok(result)
    }

    pub fn get_platform_stats(&self) -> ApiResponse<PlatformStats> {
        ApiResponse::ok(PlatformStats {
            total_transactions: self.six_cases.tx_count,
            total_users: self.registry.by_owner.len() as u64,
            tx_count: self.six_cases.tx_count,
        })
    }

    pub fn register_user(&self, req: &RegisterUserRequest) -> RegisterUserResponse {
        let short = if req.phone.len() > 4 { &req.phone[..4] } else { &req.phone };
        RegisterUserResponse { success: true, uq_number: format!("UQ-{}", short), email_uq: format!("{}@ufundi.uq", req.name.to_lowercase()) }
    }

    pub fn recover_user(&self, req: &RecoverUserRequest) -> RecoverUserResponse {
        let valid = req.fragments.len() >= 3;
        let short = if req.uq_number.len() > 6 { &req.uq_number[..6] } else { &req.uq_number };
        RecoverUserResponse { success: valid, new_uq_number: if valid { format!("UQ-RECOVER-{}", short) } else { String::new() } }
    }

    pub fn verify_otp(&self, req: &VerifyOtpRequest) -> VerifyOtpResponse {
        VerifyOtpResponse { success: true, verified: req.code.len() == 6 }
    }
}

