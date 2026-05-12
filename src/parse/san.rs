use anyhow::{anyhow, Result};
use std::collections::BTreeSet;
use x509_parser::prelude::*;

pub fn extract_sans_from_der(der: &[u8]) -> Result<Vec<String>> {
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| anyhow!("x509 parse: {e}"))?;
    let mut out = Vec::new();
    for ext in cert.extensions() {
        if let ParsedExtension::SubjectAlternativeName(san) = ext.parsed_extension() {
            for gn in &san.general_names {
                if let GeneralName::DNSName(d) = gn {
                    out.push(d.to_string());
                }
            }
        }
    }
    // Also include CN if no SANs present (rare edge case).
    if out.is_empty() {
        if let Some(cn) = cert.subject().iter_common_name().next() {
            if let Ok(s) = cn.as_str() {
                out.push(s.to_string());
            }
        }
    }
    Ok(normalize_sans(out))
}

pub fn normalize_sans(mut names: Vec<String>) -> Vec<String> {
    let mut set = BTreeSet::new();
    for mut n in names.drain(..) {
        n = n.trim().to_ascii_lowercase();
        while n.ends_with('.') {
            n.pop();
        }
        if n.is_empty() {
            continue;
        }
        set.insert(n);
    }
    set.into_iter().collect()
}
