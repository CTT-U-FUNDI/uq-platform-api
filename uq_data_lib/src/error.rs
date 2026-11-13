#[derive(Debug, Clone)]
pub enum ChainError {
    InternalError(String),
    InsufficientFunds(String),
    NetworkError(String),
}

