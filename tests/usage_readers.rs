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
            "codex_bengalfox": {
                "limitName": "GPT-5.3-Codex-Spark",
                "primary": {"usedPercent": 95, "windowDurationMins": 10080}
            },
        },
    }});
    let limits = codex::parse(&response).unwrap();

    // No window length, and the flat view must not win over the keyed one.
    assert_eq!(limits[0].label, "Usage window");
    assert_eq!(limits[1].label, "Spark Weekly");
    // Spark is displayed separately and does not replace regular Codex quota.
    assert_eq!(
        Usage {
            name: "Codex",
            limits,
        }
        .remaining_percent(),
        Some(20)
    );
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

#[test]
fn tray_is_compact_but_expanded_lines_show_every_window() {
    let usages = [
        Usage {
            name: "Codex",
            limits: vec![
                gauge::Limit {
                    label: "Weekly".into(),
                    used_percent: 80.0,
                },
                gauge::Limit {
                    label: "Spark Weekly".into(),
                    used_percent: 95.0,
                },
            ],
        },
        Usage {
            name: "Claude",
            limits: vec![
                gauge::Limit {
                    label: "5-hour".into(),
                    used_percent: 10.0,
                },
                gauge::Limit {
                    label: "Weekly".into(),
                    used_percent: 90.0,
                },
            ],
        },
    ];

    assert_eq!(gauge::tray_summary(&usages), "Codex 20%, Claude 90%");
    assert_eq!(
        gauge::quota_groups(&usages),
        [
            (
                "Codex",
                vec![
                    "  Weekly  ▰▰▱▱▱▱▱▱   20%".to_string(),
                    "  Spark\u{2006}  ▱▱▱▱▱▱▱▱    5%".to_string(),
                ],
            ),
            (
                "Claude",
                vec![
                    "  5-hour  ▰▰▰▰▰▰▰▱   90%".to_string(),
                    "  Weekly  ▰▱▱▱▱▱▱▱   10%".to_string(),
                ],
            ),
        ]
    );
}
