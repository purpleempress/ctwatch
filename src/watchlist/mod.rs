pub mod file_source;
pub mod matcher;
pub mod url_source;

use sqlx::PgPool;
use std::path::PathBuf;
use std::time::Duration;

pub use matcher::Matcher;

/// Periodically reloads the matcher from the `watchlist` table.
pub fn spawn_db_reload(pool: PgPool, matcher: Matcher, interval: Duration) {
    tokio::spawn(async move {
        loop {
            match crate::store::watchlist::list(&pool).await {
                Ok(domains) => matcher.replace(domains).await,
                Err(e) => tracing::warn!("watchlist reload: {e}"),
            }
            tokio::time::sleep(interval).await;
        }
    });
}

pub fn spawn_file_reload(matcher: Matcher, path: PathBuf, interval: Duration) {
    tokio::spawn(async move {
        loop {
            match file_source::load(&path) {
                Ok(domains) => matcher.replace(domains).await,
                Err(e) => tracing::warn!(?path, "watchlist file: {e:#}"),
            }
            tokio::time::sleep(interval).await;
        }
    });
}

pub fn spawn_url_reload(matcher: Matcher, url: String, interval: Duration) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest builds");
        loop {
            match url_source::fetch(&client, &url).await {
                Ok(domains) => matcher.replace(domains).await,
                Err(e) => tracing::warn!(%url, "watchlist URL: {e:#}"),
            }
            tokio::time::sleep(interval).await;
        }
    });
}
