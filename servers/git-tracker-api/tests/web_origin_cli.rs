// SPDX-License-Identifier: FSL-1.1-ALv2
//! Checks startup origin validation before the server attempts to connect to PostgreSQL.
use std::process::{Command, Output};

fn serve_with_origin(origin: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-tracker-api"))
        .arg("serve")
        .env_clear()
        .env("WEB_ORIGIN", origin)
        .env("DATABASE_URL", "invalid-database-url")
        .output()
        .expect("run git-tracker-api serve")
}

#[test]
fn rejects_malformed_origins_before_database_connection() {
    for origin in ["http://127.0.0.1:46320/", "https://example.com/app"] {
        let output = serve_with_origin(origin);
        assert!(!output.status.success(), "{origin}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("WEB_ORIGIN must be one exact http or https origin"),
            "{origin}: {stderr}"
        );
        assert!(
            !stderr.contains("database unavailable"),
            "{origin}: {stderr}"
        );
    }
}

#[test]
fn accepts_exact_origins_before_database_connection() {
    for origin in ["http://127.0.0.1:46320", "https://git-tracker.delino.io"] {
        let output = serve_with_origin(origin);
        assert!(!output.status.success(), "{origin}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("database unavailable"),
            "{origin}: {stderr}"
        );
        assert!(!stderr.contains("WEB_ORIGIN must"), "{origin}: {stderr}");
    }
}
