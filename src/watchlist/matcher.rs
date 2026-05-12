use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Default, Clone)]
pub struct Matcher {
    inner: Arc<RwLock<HashSet<String>>>,
}

impl Matcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn replace(&self, domains: impl IntoIterator<Item = String>) {
        let mut w = self.inner.write().await;
        w.clear();
        for d in domains {
            w.insert(d.to_ascii_lowercase());
        }
    }

    pub async fn matches(&self, registered_domains: &[String]) -> Vec<String> {
        let r = self.inner.read().await;
        registered_domains
            .iter()
            .filter(|d| r.contains(&d.to_ascii_lowercase()))
            .cloned()
            .collect()
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}
