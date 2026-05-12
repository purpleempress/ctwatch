use ctwatch::parse::{extract_sans_from_der, normalize_sans, registered_domain};

#[test]
fn extracts_sans_from_real_precert_der() {
    let der = std::fs::read("tests/fixtures/precert_sample.der").unwrap();
    let sans = extract_sans_from_der(&der).expect("extract");
    assert!(
        !sans.is_empty(),
        "real precert should have at least one SAN"
    );
}

#[test]
fn normalize_sans_lowercases_dedups_sorts_strips_trailing_dot() {
    let input = vec![
        "WWW.Example.COM.".to_string(),
        "www.example.com".to_string(),
        "api.example.com".to_string(),
        "*.example.com".to_string(),
    ];
    let normed = normalize_sans(input);
    assert_eq!(
        normed,
        vec!["*.example.com", "api.example.com", "www.example.com"]
    );
}

#[test]
fn registered_domain_via_psl() {
    assert_eq!(
        registered_domain("www.example.com").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        registered_domain("example.co.uk").as_deref(),
        Some("example.co.uk")
    );
    assert_eq!(
        registered_domain("foo.example.co.uk").as_deref(),
        Some("example.co.uk")
    );
    assert_eq!(
        registered_domain("*.example.com").as_deref(),
        Some("example.com")
    );
    assert_eq!(registered_domain("localhost").as_deref(), None);
}
