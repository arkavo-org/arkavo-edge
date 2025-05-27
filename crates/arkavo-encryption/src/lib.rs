pub fn encrypt(data: &[u8], _key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(data.to_vec())
}

pub fn decrypt(data: &[u8], _key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(data.to_vec())
}
