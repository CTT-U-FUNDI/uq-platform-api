use serde::{Serialize, Deserialize, Deserializer, Serializer, de};
use sha2::{Sha256, Digest};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    #[serde(serialize_with = "hex_ser_32", deserialize_with = "hex_de_32")]
    pub id: [u8; 32],
    #[serde(serialize_with = "hex_ser_32", deserialize_with = "hex_de_32")]
    pub from: [u8; 32],
    #[serde(serialize_with = "hex_ser_32", deserialize_with = "hex_de_32")]
    pub to: [u8; 32],
    pub amount: u64,
    pub fee: u64,
    pub nonce: u64,
    pub timestamp: i64,
    pub signature: Vec<u8>,
}

fn hex_ser_32<S>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> where S: Serializer {
    s.serialize_str(&hex::encode(bytes))
}

fn hex_de_32<'de, D>(d: D) -> Result<[u8; 32], D::Error> where D: Deserializer<'de> {
    let s = String::deserialize(d)?;
    let bytes = hex::decode(&s).map_err(de::Error::custom)?;
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

impl Transaction {
    pub fn new(from: [u8; 32], to: [u8; 32], amount: u64, fee: u64, nonce: u64) -> Self {
        let mut tx = Transaction {
            id: [0u8; 32], from, to, amount, fee, nonce,
            timestamp: chrono::Utc::now().timestamp(),
            signature: vec![],
        };
        tx.id = tx.compute_hash();
        tx
    }
    
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.from);
        hasher.update(&self.to);
        hasher.update(&self.amount.to_le_bytes());
        hasher.update(&self.fee.to_le_bytes());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.timestamp.to_le_bytes());
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }
}






