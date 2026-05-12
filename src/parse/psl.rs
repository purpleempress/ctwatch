use publicsuffix::{List, Psl};

static LIST_BYTES: &[u8] = include_bytes!("../../public_suffix_list.dat");

fn list() -> &'static List {
    use std::sync::OnceLock;
    static INSTANCE: OnceLock<List> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        std::str::from_utf8(LIST_BYTES)
            .unwrap()
            .parse()
            .expect("PSL parses")
    })
}

/// Returns the eTLD+1 (registered domain) for the input. Strips a leading "*."
/// before computing. Returns None for non-public-suffix names like "localhost".
pub fn registered_domain(name: &str) -> Option<String> {
    let stripped = name.strip_prefix("*.").unwrap_or(name);
    let bytes = stripped.as_bytes();
    let dom = list().domain(bytes)?;
    Some(std::str::from_utf8(dom.as_bytes()).ok()?.to_string())
}
