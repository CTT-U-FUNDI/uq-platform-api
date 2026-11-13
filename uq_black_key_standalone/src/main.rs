use uq_black_key_standalone::UqBlackKey;

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

fn main() {
    let key = UqBlackKey::generate();
    let message = b"UQ production key demo";
    let signature = key.sign(message);
    let ciphertext = key.encrypt(message);
    let ciphertext = key.encrypt(message).unwrap();
    let decrypted = key.decrypt(&ciphertext).unwrap();

    println!("address={}", hex_encode(&key.address));
    println!("public_key_len={}", key.public_key.len());
    println!("signature_valid={}", key.verify(message, &signature));
    println!("decrypt_valid={}", decrypted == message.to_vec());
    println!("integrity_valid={}", key.verify_integrity());
}