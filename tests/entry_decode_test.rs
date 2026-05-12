use base64::Engine;
use ctwatch::ingest::entry::{decode_leaf, EntryKind};

#[test]
fn decodes_real_precert_leaf() {
    let leaf_b64 = std::fs::read_to_string("tests/fixtures/precert_leaf_input.b64").unwrap();
    let extra_b64 = std::fs::read_to_string("tests/fixtures/precert_extra_data.b64").unwrap();
    let leaf = base64::engine::general_purpose::STANDARD
        .decode(leaf_b64.trim())
        .unwrap();
    let extra = base64::engine::general_purpose::STANDARD
        .decode(extra_b64.trim())
        .unwrap();

    let entry = decode_leaf(&leaf, &extra).expect("decode");
    match entry.kind {
        EntryKind::Precert {
            issuer_key_hash, ..
        } => {
            assert_eq!(issuer_key_hash.len(), 32);
        }
        EntryKind::FinalCert { .. } => panic!("fixture should be precert"),
    }
    assert!(
        entry.precert_der.is_some(),
        "precert DER extracted from extra_data"
    );
    assert!(entry.timestamp_ms > 0);
}
