use gauge::{claude, codex, Usage};
use serde_json::json;

fn usage(limits: Vec<gauge::Limit>) -> Usage {
    Usage {
        name: "Test",
        limits,
    }
}

#[test]
fn codex_reports_remaining_quota() {
    let response = json!({"id": 2, "result": {"rateLimits": {
        "primary": {"usedPercent": 19, "windowDurationMins": 10080},
        "secondary": null,
    }}});
    let limits = codex::parse(&response).unwrap();

    assert_eq!(limits.len(), 1);
    assert_eq!(limits[0].label, "Weekly");
    assert_eq!(usage(limits).remaining_percent(), Some(81));
}

#[test]
fn codex_prefers_the_codex_bucket_of_multi_limit_accounts() {
    let response = json!({"id": 2, "result": {
        "rateLimits": {"primary": {"usedPercent": 5}},
        "rateLimitsByLimitId": {
            "codex_other": {"primary": {"usedPercent": 12}},
            "codex": {"primary": {"usedPercent": 80}},
        },
    }});
    let limits = codex::parse(&response).unwrap();

    // No window length, and the flat view must not win over the keyed one.
    assert_eq!(limits[0].label, "Usage window");
    assert_eq!(usage(limits).remaining_percent(), Some(20));
}

#[test]
fn codex_surfaces_protocol_errors() {
    let response = json!({"id": 2, "error": {"message": "not signed in"}});
    assert!(codex::parse(&response).is_err());
    assert!(codex::parse(&json!({"id": 2, "result": {}})).is_err());
}

#[test]
fn claude_reads_every_metered_window() {
    let body = br#"{"limits": [
        {"kind": "session", "percent": 34},
        {"kind": "weekly_all", "percent": 35},
        {"kind": "weekly_opus", "percent": 4}
    ]}"#;
    let limits = claude::parse(body).unwrap();

    assert_eq!(limits.len(), 3);
    assert_eq!(limits[0].label, "5-hour");
    assert_eq!(limits[2].label, "Weekly (Opus)");

    // The headline follows the tightest window, not the largest remainder.
    assert_eq!(usage(limits).remaining_percent(), Some(65));
}

#[test]
fn claude_rejects_a_payload_without_windows() {
    assert!(claude::parse(br#"{"limits": []}"#).is_err());
    assert!(claude::parse(b"not json").is_err());
}

#[test]
fn summary_notes_when_nothing_could_be_read() {
    assert_eq!(gauge::summary(&[]), "Agent quota unavailable");
}
