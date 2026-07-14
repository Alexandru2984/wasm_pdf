use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::error::AuthError;

pub async fn hash_password(password: String) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || {
        let salt =
            SaltString::encode_b64(Uuid::new_v4().as_bytes()).map_err(AuthError::internal)?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(AuthError::internal)
    })
    .await
    .map_err(AuthError::internal)?
}

pub async fn verify_password(password: String, encoded_hash: String) -> Result<bool, AuthError> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&encoded_hash).map_err(AuthError::internal)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .map_err(AuthError::internal)?
}

pub fn random_token() -> Result<String, AuthError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(AuthError::internal)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn generate_backup_code() -> String {
    const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let compact = bytes
        .iter()
        .enumerate()
        .filter(|(index, _)| !matches!(index, 6 | 8 | 22 | 24))
        .take(20)
        .map(|(_, byte)| char::from(ALPHABET[usize::from(*byte & 31)]))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &compact[..4],
        &compact[4..8],
        &compact[8..12],
        &compact[12..16],
        &compact[16..]
    )
}

pub fn normalize_backup_code(value: &str) -> Option<String> {
    let normalized = value
        .chars()
        .filter(|character| *character != '-' && !character.is_ascii_whitespace())
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    (normalized.len() == 20
        && normalized
            .bytes()
            .all(|byte| b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ".contains(&byte)))
    .then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_codes_are_readable_and_normalizable() {
        let code = generate_backup_code();
        assert_eq!(code.len(), 24);
        assert_eq!(normalize_backup_code(&code).expect("valid code").len(), 20);
        assert!(normalize_backup_code("invalid-code").is_none());
    }

    #[test]
    fn opaque_tokens_use_32_random_bytes() {
        let first = random_token().expect("random token");
        let second = random_token().expect("random token");
        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
    }
}
