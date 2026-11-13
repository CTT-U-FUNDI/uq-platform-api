use crate::crypto::sha256;
use crate::error::UqError;

pub const ADDRESS_SIZE: usize = 32;
pub const ADDRESS_PREFIX: &str = "uq:";
const CHECKSUM_SIZE: usize = 4;
const PAYLOAD_SIZE: usize = ADDRESS_SIZE - CHECKSUM_SIZE;
pub const ADDRESS_STRING_LENGTH: usize = 3 + 64;

/// Crée une adresse UQ à partir d'une clé publique Ed25519.
///
/// # Exemple
/// ```ignore
/// let addr = create_address(&pubkey, 1720000000, &device_id)?;
/// ```
pub fn create_address(
    public_key: &[u8; 32],
    timestamp: i64,
    device_id: &[u8; 32],
) -> Result<String, UqError> {
    let pk_hash = sha256(public_key);
    let seg1: [u8; 8] = pk_hash[0..8].try_into().unwrap_or_default();

    let mut seed_input = Vec::with_capacity(39);
    seed_input.extend_from_slice(public_key);
    seed_input.extend_from_slice(b"UQ_SEED_V1");
    let seed_hash = sha256(&seed_input);
    let seg2: [u8; 8] = seed_hash[0..8].try_into().unwrap_or_default();

    let time_hash = sha256(&timestamp.to_le_bytes());
    let seg3: [u8; 8] = time_hash[0..8].try_into().unwrap_or_default();

    let _flags: u32 = 0x00000001;

    let device_hash = sha256(device_id);
    let seg5: [u8; 4] = device_hash[0..4].try_into().unwrap_or_default();

    let mut payload = [0u8; PAYLOAD_SIZE];
    payload[0..8].copy_from_slice(&seg1);
    payload[8..16].copy_from_slice(&seg2);
    payload[16..24].copy_from_slice(&seg3);
    payload[24..28].copy_from_slice(&seg5);

    let checksum_full = sha256(&payload);
    let checksum: [u8; CHECKSUM_SIZE] = checksum_full[0..CHECKSUM_SIZE].try_into().unwrap_or_default();

    let mut address_bytes = [0u8; ADDRESS_SIZE];
    address_bytes[0..PAYLOAD_SIZE].copy_from_slice(&payload);
    address_bytes[PAYLOAD_SIZE..].copy_from_slice(&checksum);

    Ok(format!("{}{}", ADDRESS_PREFIX, hex::encode(address_bytes)))
}

pub fn is_valid(address: &str) -> Result<bool, UqError> {
    if !address.starts_with(ADDRESS_PREFIX) { return Ok(false); }
    let hex_part = &address[ADDRESS_PREFIX.len()..];
    if hex_part.len() != 64 { return Ok(false); }
    if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) { return Ok(false); }
    let bytes = match hex::decode(hex_part) {
        Ok(b) if b.len() == ADDRESS_SIZE => b,
        _ => return Ok(false),
    };
    let payload = &bytes[0..PAYLOAD_SIZE];
    let stored = &bytes[PAYLOAD_SIZE..];
    let computed = &sha256(payload)[0..CHECKSUM_SIZE];
    Ok(stored == computed)
}

pub fn to_bytes(address: &str) -> Result<[u8; ADDRESS_SIZE], UqError> {
    if !address.starts_with(ADDRESS_PREFIX) {
        return Err(UqError::InvalidAddress("L'adresse doit commencer par 'uq:'".to_string()));
    }
    let hex_part = &address[ADDRESS_PREFIX.len()..];
    let bytes = hex::decode(hex_part)
        .map_err(|e| UqError::InvalidAddress(format!("Décodage hex échoué: {}", e)))?;
    if bytes.len() != ADDRESS_SIZE {
        return Err(UqError::InvalidAddress(format!("Taille invalide: {}", bytes.len())));
    }
    let mut result = [0u8; ADDRESS_SIZE];
    result.copy_from_slice(&bytes);
    Ok(result)
}

pub fn to_string(bytes: &[u8; ADDRESS_SIZE]) -> String {
    format!("{}{}", ADDRESS_PREFIX, hex::encode(bytes))
}

pub fn get_flags(address: &str) -> Result<u32, UqError> {
    let bytes = to_bytes(address)?;
    let flag_bytes: [u8; 4] = bytes[24..28].try_into().unwrap_or_default();
    Ok(u32::from_le_bytes(flag_bytes))
}

pub fn is_validator(address: &str) -> Result<bool, UqError> {
    Ok((get_flags(address)? & 0x02) != 0)
}

pub fn is_relay(address: &str) -> Result<bool, UqError> {
    Ok((get_flags(address)? & 0x04) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    fn test_address() -> String {
        let kp = generate_keypair();
        let device_id = sha256(b"test-device-001");
        create_address(&kp.public_key, 1720000000, &device_id).unwrap_or_default()
    }

    #[test]
    fn test_create_address_format() {
        let addr = test_address();
        assert!(addr.starts_with("uq:"));
        assert_eq!(addr.len(), ADDRESS_STRING_LENGTH);
    }

    #[test]
    fn test_is_valid_positive() { assert!(is_valid(&test_address()).unwrap_or_default()); }
    
    #[test]
    fn test_is_valid_negative() {
        assert!(!is_valid("").unwrap_or_default());
        assert!(!is_valid("uq:zzzz").unwrap_or_default());
        assert!(!is_valid("bad:abcd").unwrap_or_default());
    }

    #[test]
    fn test_bytes_roundtrip() {
        let addr = test_address();
        let bytes = to_bytes(&addr).unwrap_or_default();
        assert_eq!(addr, to_string(&bytes));
    }

    #[test]
    fn test_different_keys() {
        let kp1 = generate_keypair();
        let kp2 = generate_keypair();
        let did = sha256(b"test");
        assert_ne!(
            create_address(&kp1.public_key, 1720000000, &did).unwrap_or_default(),
            create_address(&kp2.public_key, 1720000000, &did).unwrap_or_default()
        );
    }

    #[test]
    fn test_checksum_detects_corruption() {
        let addr = test_address();
        let mut bytes = to_bytes(&addr).unwrap_or_default();
        bytes[10] ^= 0xFF;
        assert!(!is_valid(&to_string(&bytes)).unwrap_or_default());
    }
}
