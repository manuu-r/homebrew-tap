use gauge::provisioning::{parse_identity, COMMISSION_PROTOCOL, COMMISSION_VERSION};
use serde_json::Value;

const IDENTITY: &str = include_str!("../docs/fixtures/accessory-identity-v1.json");
const COMMISSION: &str = include_str!("../docs/fixtures/commission-v1.json");
const DASHBOARD: &str = include_str!("../docs/fixtures/dashboard-v1.json");

#[test]
fn reference_identity_is_vendor_neutral_and_accepted() {
    let identity = parse_identity(IDENTITY.as_bytes()).unwrap();
    assert_eq!(identity.device_id, "acme-desk-a1b2c3");
    assert_eq!(identity.name.as_deref(), Some("Acme Desk Display"));
    assert_eq!(identity.kind.as_deref(), Some("display"));
    assert_eq!(identity.firmware_version.as_deref(), Some("2.3.1"));
    assert!(identity
        .capabilities
        .iter()
        .any(|capability| capability == "dashboard.pull"));
}

#[test]
fn reference_commission_document_fits_minimum_mtu_framing() {
    let document: Value = serde_json::from_str(COMMISSION).unwrap();
    assert_eq!(document["protocol"], COMMISSION_PROTOCOL);
    assert_eq!(document["version"], COMMISSION_VERSION);
    assert_eq!(document["accessory"]["protocol"], "dev.gauge.accessory");
    assert_eq!(document["accessory"]["version"], 1);
    assert_eq!(
        document["accessory"]["bearer_token"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    let compact = serde_json::to_vec(&document).unwrap();
    assert!(compact.len().div_ceil(17) <= u8::MAX as usize);
}

#[test]
fn reference_dashboard_has_every_version_one_section() {
    let document: Value = serde_json::from_str(DASHBOARD).unwrap();
    assert_eq!(document["protocol"], "dev.gauge.dashboard");
    assert_eq!(document["schema_version"], 1);
    assert!(document["refresh_seconds"].is_u64());
    assert!(document["quota"]["providers"].is_array());
    assert!(document["calendar"]["events"].is_array());
    assert!(document["todos"].is_array());
}
