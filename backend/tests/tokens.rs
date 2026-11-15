//! Verifies: TEAM.SYS.PKG-R05. Verifiers never contain reusable bearer credentials.
use teamarness_generated_package_backend::tokens::{
    Entropy, EntropyError, TokenRecord, issue, verifier, verify,
};
struct Fixed;
impl Entropy for Fixed {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), EntropyError> {
        bytes.fill(0);
        Ok(())
    }
}
struct Broken;
impl Entropy for Broken {
    fn fill(&self, _: &mut [u8]) -> Result<(), EntropyError> {
        Err(EntropyError)
    }
}
#[test]
fn fixed_vector_is_domain_and_generation_bound() {
    // Independently computed SHA-256(domain || id bytes || generation BE || secret bytes).
    assert_eq!(
        verifier(&[0; 16], 1, &[0; 32]),
        "01cf874773618ae1897f81299acfe1df02d2d5bdc55e164d77cf70e151247d44"
    );
    let (record, secret) = issue("client", 1, None, &Fixed).unwrap();
    assert!(verify(&record, &secret));
    let other = TokenRecord {
        generation: 2,
        ..record.clone()
    };
    assert!(!verify(&other, &secret));
    assert!(!verify(&record, &(secret.clone() + "=")));
    assert!(!serde_json::to_string(&record).unwrap().contains(&secret));
    assert!(issue("client", 1, None, &Broken).is_err());
    assert!(issue(&"a".repeat(97), 1, None, &Fixed).is_err());
    assert!(issue(&"a".repeat(96), 1, None, &Fixed).is_ok());
}
