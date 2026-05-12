// tests/e2e_test.rs — runs only when CTWATCH_E2E=1
use std::process::{Command, Stdio};
use std::time::Duration;

fn e2e_enabled() -> bool {
    std::env::var("CTWATCH_E2E").ok().as_deref() == Some("1")
}

#[test]
fn full_stack_ingests_and_responds() {
    if !e2e_enabled() {
        eprintln!("skipping: set CTWATCH_E2E=1 to run");
        return;
    }
    // Assumes Postgres is at $DATABASE_URL, ctwatch binary at $CTWATCH_BIN.
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let bin = std::env::var("CTWATCH_BIN").unwrap_or_else(|_| "./target/release/ctwatch".into());

    // Migrate.
    let status = Command::new(&bin)
        .arg("migrate")
        .env("DATABASE_URL", &database_url)
        .status()
        .unwrap();
    assert!(status.success());

    // Boot serve in background.
    let mut serve = Command::new(&bin)
        .arg("serve")
        .env("DATABASE_URL", &database_url)
        .env("LISTEN_ADDR", "127.0.0.1:18080")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait up to 120s for /v1/stats to show non-zero ingest rate.
    let start = std::time::Instant::now();
    let mut ok = false;
    while start.elapsed() < Duration::from_secs(120) {
        if let Ok(resp) = reqwest::blocking::get("http://127.0.0.1:18080/v1/stats") {
            if resp.status().is_success() {
                let v: serde_json::Value = resp.json().unwrap();
                let rate = v["ingest"]["precerts_per_sec_5m"].as_f64().unwrap_or(0.0);
                let certs = v["totals"]["certs_in_window"].as_i64().unwrap_or(0);
                if certs > 0 && rate > 0.0 {
                    ok = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }

    let _ = serve.kill();
    assert!(ok, "expected non-zero ingest within 120s");
}
