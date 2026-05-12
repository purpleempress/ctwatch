use ctwatch::ingest::log_list::{parse_log_list, LogState};

#[test]
fn parses_real_log_list_fixture_and_filters_usable() {
    let raw =
        std::fs::read_to_string("tests/fixtures/log_list_sample.json").expect("fixture missing");
    let parsed = parse_log_list(&raw).expect("parse");
    let usable: Vec<_> = parsed
        .into_iter()
        .filter(|l| l.state == LogState::Usable)
        .collect();
    assert!(!usable.is_empty(), "should have at least one usable log");
    // Sanity: each log has a 32-byte log_id (decoded from base64).
    for l in &usable {
        assert_eq!(
            l.log_id.len(),
            32,
            "log_id should be 32 bytes for {}",
            l.url
        );
    }
}
