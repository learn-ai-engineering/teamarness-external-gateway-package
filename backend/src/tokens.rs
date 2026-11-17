//! High-entropy bearer credentials. Only verifiers are durable; labels confer no authority.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Debug, Clone, Copy)]
pub struct EntropyError;
pub trait Entropy: Send + Sync {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), EntropyError>;
}
pub struct OsEntropy;
impl Entropy for OsEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), EntropyError> {
        getrandom::fill(bytes).map_err(|_| EntropyError)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenRecord {
    pub id: String,
    pub label: String,
    pub generation: u64,
    pub verifier: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    Invalid,
    Entropy,
}
pub fn verifier(id: &[u8; 16], generation: u64, secret: &[u8; 32]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"external-gateway.token.v1\0");
    hash.update(id);
    hash.update(generation.to_be_bytes());
    hash.update(secret);
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn issue(
    label: &str,
    generation: u64,
    id: Option<&str>,
    entropy: &dyn Entropy,
) -> Result<(TokenRecord, String), TokenError> {
    if label.is_empty() || label.len() > 96 || generation == 0 {
        return Err(TokenError::Invalid);
    }
    let mut id_bytes = [0; 16];
    if let Some(id) = id {
        id_bytes = decode(id).ok_or(TokenError::Invalid)?;
    } else {
        entropy
            .fill(&mut id_bytes)
            .map_err(|_| TokenError::Entropy)?;
    }
    let mut secret = [0; 32];
    entropy.fill(&mut secret).map_err(|_| TokenError::Entropy)?;
    let id = URL_SAFE_NO_PAD.encode(id_bytes);
    let bearer = format!("eg1.{id}.{}", URL_SAFE_NO_PAD.encode(secret));
    Ok((
        TokenRecord {
            verifier: verifier(&id_bytes, generation, &secret),
            id,
            label: label.to_owned(),
            generation,
        },
        bearer,
    ))
}
fn decode<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    if URL_SAFE_NO_PAD.encode(&bytes) != value {
        return None;
    }
    bytes.try_into().ok()
}
pub(crate) fn valid_id(id: &str) -> bool {
    decode::<16>(id).is_some()
}
impl TokenRecord {
    pub fn valid(&self) -> bool {
        valid_id(&self.id)
            && self.generation > 0
            && !self.label.is_empty()
            && self.label.len() <= 96
            && self.verifier.len() == 64
            && self
                .verifier
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }
}
pub fn verify(record: &TokenRecord, bearer: &str) -> bool {
    if bearer.len() != 70 {
        return false;
    }
    let mut parts = bearer.split('.');
    if parts.next() != Some("eg1") {
        return false;
    }
    let Some(id) = parts.next() else {
        return false;
    };
    let Some(secret) = parts.next().and_then(decode::<32>) else {
        return false;
    };
    let Some(id_bytes) = decode::<16>(id) else {
        return false;
    };
    if parts.next().is_some() || id != record.id || !record.valid() {
        return false;
    }
    let expected = verifier(&id_bytes, record.generation, &secret);
    bool::from(expected.as_bytes().ct_eq(record.verifier.as_bytes()))
}
