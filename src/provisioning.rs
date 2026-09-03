//! Native Bluetooth commissioning for Gauge-compatible accessories.
//!
//! Sensitive GATT characteristics require authenticated encryption. On macOS,
//! the first read therefore invokes the system Bluetooth Secure Connections
//! numeric-comparison sheet; Gauge never implements or duplicates that UI.
//! Device-specific controls for accepting that comparison stay entirely on
//! the accessory.

use btleplug::{
    api::{Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType},
    platform::{Manager, Peripheral},
};
use serde::Deserialize;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

use crate::devices::{DeviceStore, PairedDevice};

pub const PAIRING_SERVICE_UUID: Uuid = Uuid::from_u128(0xc9cc_e6f3_bf10_4e6d_b719_f329_11bb_ba89);
const IDENTITY_CHARACTERISTIC_UUID: Uuid =
    Uuid::from_u128(0x1db9_634c_a20f_43c6_8ed9_69ce_da33_8178);
const CONFIG_CHARACTERISTIC_UUID: Uuid = Uuid::from_u128(0x289b_e295_d110_411b_888c_c80a_6011_77fa);
const STATUS_CHARACTERISTIC_UUID: Uuid = Uuid::from_u128(0xe763_eccb_fa4c_4e3a_9211_8505_1337_1105);

pub const PAIRING_PROTOCOL: &str = "dev.gauge.pairing";
pub const PAIRING_VERSION: u16 = 1;
pub const COMMISSION_PROTOCOL: &str = "dev.gauge.commission";
pub const COMMISSION_VERSION: u16 = 1;
const SCAN_TIME: Duration = Duration::from_secs(7);
const CONNECT_TIME: Duration = Duration::from_secs(15);
const DISCOVERY_TIME: Duration = Duration::from_secs(12);
const USER_CONFIRM_TIME: Duration = Duration::from_secs(45);
const WIFI_JOIN_TIME: Duration = Duration::from_secs(45);

// The default ATT MTU permits a 20-byte value. Three framing bytes leave 17
// bytes of payload and avoid depending on an MTU negotiation hidden by
// CoreBluetooth.
const FRAME_MAGIC: u8 = 0x47;
const FRAME_HEADER_SIZE: usize = 3;
const FRAME_DATA_SIZE: usize = 20 - FRAME_HEADER_SIZE;
const MAX_FRAME_COUNT: usize = u8::MAX as usize;

#[derive(Clone, Debug)]
pub struct PairingRequest {
    pub wifi_ssid: String,
    pub wifi_password: String,
}

impl PairingRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wifi_ssid.trim().is_empty() || self.wifi_ssid.len() > 32 {
            return Err("Wi-Fi name must be 1-32 bytes".into());
        }
        if self.wifi_password.len() > 64
            || (self.wifi_password.len() == 64
                && !self
                    .wifi_password
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("Wi-Fi password must be at most 63 bytes or a 64-digit hex key".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AccessoryIdentity {
    pub protocol: String,
    pub version: u16,
    pub device_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub firmware_version: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Pair one nearby compatible accessory and send its Wi-Fi and Gauge server
/// configuration.
/// This blocking entry point is intended for the menu app's worker thread.
pub fn pair_accessory(
    devices: Arc<DeviceStore>,
    request: PairingRequest,
    server_port: u16,
) -> Result<PairedDevice, String> {
    request.validate()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("gauge-bluetooth")
        .build()
        .map_err(|error| format!("could not start Bluetooth setup: {error}"))?;
    runtime.block_on(pair_accessory_async(devices, request, server_port))
}

async fn pair_accessory_async(
    devices: Arc<DeviceStore>,
    request: PairingRequest,
    server_port: u16,
) -> Result<PairedDevice, String> {
    let adapter = bluetooth_adapter().await?;
    adapter
        .start_scan(ScanFilter {
            services: vec![PAIRING_SERVICE_UUID],
        })
        .await
        .map_err(|error| format!("could not scan for Gauge accessories: {error}"))?;
    tokio::time::sleep(SCAN_TIME).await;
    let candidates = matching_peripherals(&adapter).await;
    let _ = adapter.stop_scan().await;
    let (peripheral, name) = select_peripheral(candidates?)?;

    peripheral
        .connect_with_timeout(CONNECT_TIME)
        .await
        .map_err(|error| format!("could not connect to {name}: {error}"))?;
    let result = provision_connected(&peripheral, &name, &devices, &request, server_port).await;
    let _ = peripheral.disconnect().await;
    result
}

async fn bluetooth_adapter() -> Result<btleplug::platform::Adapter, String> {
    let manager = Manager::new()
        .await
        .map_err(|error| format!("could not open Bluetooth: {error}"))?;
    manager
        .adapters()
        .await
        .map_err(|error| format!("could not inspect Bluetooth adapters: {error}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "this Mac has no available Bluetooth adapter".into())
}

async fn matching_peripherals(
    adapter: &btleplug::platform::Adapter,
) -> Result<Vec<(Peripheral, String)>, String> {
    let mut matches = Vec::new();
    for peripheral in adapter
        .peripherals()
        .await
        .map_err(|error| format!("could not read Bluetooth scan results: {error}"))?
    {
        let Some(properties) = peripheral
            .properties()
            .await
            .map_err(|error| format!("could not inspect a Bluetooth device: {error}"))?
        else {
            continue;
        };
        let advertises_gauge = properties.services.contains(&PAIRING_SERVICE_UUID)
            || properties.service_data.contains_key(&PAIRING_SERVICE_UUID);
        if advertises_gauge {
            let name = properties
                .local_name
                .or(properties.advertisement_name)
                .unwrap_or_else(|| "Gauge accessory".into());
            matches.push((peripheral, name));
        }
    }
    Ok(matches)
}

fn select_peripheral(
    candidates: Vec<(Peripheral, String)>,
) -> Result<(Peripheral, String), String> {
    match candidates.len() {
        0 => Err(
            "no compatible accessory in pairing mode was found; keep it nearby and try again"
                .into(),
        ),
        1 => Ok(candidates.into_iter().next().unwrap()),
        _ => Err(
            "more than one compatible accessory is in pairing mode; leave only one active and retry"
                .into(),
        ),
    }
}

async fn provision_connected(
    peripheral: &Peripheral,
    name: &str,
    devices: &DeviceStore,
    request: &PairingRequest,
    server_port: u16,
) -> Result<PairedDevice, String> {
    peripheral
        .discover_services_with_timeout(DISCOVERY_TIME)
        .await
        .map_err(|error| format!("could not discover {name}'s pairing service: {error}"))?;
    let identity_characteristic = characteristic(peripheral, IDENTITY_CHARACTERISTIC_UUID)?;
    let config_characteristic = characteristic(peripheral, CONFIG_CHARACTERISTIC_UUID)?;
    let status_characteristic = characteristic(peripheral, STATUS_CHARACTERISTIC_UUID)?;

    // Reading this MITM-protected characteristic is the only trigger needed:
    // macOS owns the numeric-comparison sheet and the ESP32 waits for a tap.
    let identity_body =
        tokio::time::timeout(USER_CONFIRM_TIME, peripheral.read(&identity_characteristic))
            .await
            .map_err(|_| {
                "Bluetooth confirmation timed out; confirm on the Mac and on the accessory"
                    .to_string()
            })?
            .map_err(|_| {
                "secure pairing was cancelled or the numbers were not confirmed on both devices"
                    .to_string()
            })?;
    let identity = parse_identity(&identity_body)?;

    let accessory_name = identity.name.as_deref().unwrap_or(name);
    let accessory_kind = identity.kind.as_deref().unwrap_or_else(|| {
        if identity
            .capabilities
            .iter()
            .any(|capability| capability == "display")
        {
            "display"
        } else {
            "accessory"
        }
    });
    let bundle = devices.pair(
        &identity.device_id,
        accessory_name,
        accessory_kind,
        identity.firmware_version.as_deref(),
        identity.capabilities,
        server_port,
    )?;
    let device_id = bundle.device_id.clone();
    let result = async {
        let payload = serde_json::to_vec(&json!({
            "protocol": COMMISSION_PROTOCOL,
            "version": COMMISSION_VERSION,
            "wifi": {
                "ssid": request.wifi_ssid,
                "password": request.wifi_password,
            },
            "accessory": bundle,
        }))
        .map_err(|error| format!("could not encode accessory setup: {error}"))?;
        write_framed(peripheral, &config_characteristic, &payload).await?;
        wait_for_wifi(peripheral, &status_characteristic, accessory_name).await
    }
    .await;

    if let Err(error) = result {
        let _ = devices.revoke(&device_id);
        return Err(error);
    }

    devices
        .devices()
        .into_iter()
        .find(|device| device.id == device_id)
        .ok_or_else(|| "the paired accessory was not saved".into())
}

fn characteristic(peripheral: &Peripheral, uuid: Uuid) -> Result<Characteristic, String> {
    peripheral
        .characteristics()
        .into_iter()
        .find(|characteristic| characteristic.uuid == uuid)
        .ok_or_else(|| "the accessory does not expose the complete secure pairing service".into())
}

pub fn parse_identity(body: &[u8]) -> Result<AccessoryIdentity, String> {
    let mut identity: AccessoryIdentity = serde_json::from_slice(body)
        .map_err(|_| "accessory returned invalid secure identity data")?;
    if identity.protocol != PAIRING_PROTOCOL || identity.version != PAIRING_VERSION {
        return Err("accessory uses an unsupported pairing protocol".into());
    }
    identity.device_id = device_identifier(&identity.device_id)?;
    identity.name = clean_optional_label(identity.name, "accessory name")?;
    identity.kind = clean_optional_label(identity.kind, "accessory kind")?;
    identity.firmware_version =
        clean_optional_label(identity.firmware_version, "firmware version")?;
    identity.capabilities = normalize_capabilities(identity.capabilities)?;
    Ok(identity)
}

fn clean_optional_label(value: Option<String>, field: &str) -> Result<Option<String>, String> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() || value.len() > 80 || value.chars().any(char::is_control) {
                return Err(format!("{field} must be 1-80 printable characters"));
            }
            Ok(value.to_owned())
        })
        .transpose()
}

fn normalize_capabilities(capabilities: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::with_capacity(capabilities.len());
    for capability in capabilities {
        let capability = capability.trim().to_ascii_lowercase();
        if capability.is_empty()
            || capability.len() > 48
            || !capability.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            })
        {
            return Err("accessory returned an invalid capability identifier".into());
        }
        normalized.push(capability);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn device_identifier(value: &str) -> Result<String, String> {
    let identifier = value.trim().to_ascii_lowercase();
    if identifier.is_empty()
        || identifier.len() > 64
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("accessory returned an invalid stable device identity".into());
    }
    Ok(identifier)
}

fn frames(payload: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let count = payload.len().div_ceil(FRAME_DATA_SIZE);
    if count == 0 || count > MAX_FRAME_COUNT {
        return Err("accessory setup data is too large".into());
    }
    Ok(payload
        .chunks(FRAME_DATA_SIZE)
        .enumerate()
        .map(|(index, chunk)| {
            let mut frame = Vec::with_capacity(FRAME_HEADER_SIZE + chunk.len());
            frame.extend_from_slice(&[FRAME_MAGIC, index as u8, count as u8]);
            frame.extend_from_slice(chunk);
            frame
        })
        .collect())
}

async fn write_framed(
    peripheral: &Peripheral,
    characteristic: &Characteristic,
    payload: &[u8],
) -> Result<(), String> {
    for frame in frames(payload)? {
        peripheral
            .write(characteristic, &frame, WriteType::WithResponse)
            .await
            .map_err(|error| format!("secure accessory setup write failed: {error}"))?;
    }
    Ok(())
}

async fn wait_for_wifi(
    peripheral: &Peripheral,
    status: &Characteristic,
    name: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + WIFI_JOIN_TIME;
    while tokio::time::Instant::now() < deadline {
        match peripheral.read(status).await {
            Ok(body) => {
                let state = String::from_utf8_lossy(&body);
                let state = state.trim_matches(char::from(0)).trim();
                if state == "connected" {
                    return Ok(());
                }
                if let Some(error) = state.strip_prefix("error:") {
                    return Err(format!("{name} could not finish setup: {}", error.trim()));
                }
            }
            Err(error) => {
                return Err(format!("lost {name} while it was joining Wi-Fi: {error}"));
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    Err(format!(
        "{name} could not join Wi-Fi; check the network password and retry"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_input_rejects_invalid_wifi_values() {
        let request = PairingRequest {
            wifi_ssid: "Home".into(),
            wifi_password: "secret".into(),
        };
        assert!(request.validate().is_ok());
        let mut invalid = request;
        invalid.wifi_ssid.clear();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn minimum_mtu_frames_round_trip() {
        let payload = vec![0x5a; 100];
        let frames = frames(&payload).unwrap();
        assert!(frames.iter().all(|frame| frame.len() <= 20));
        assert_eq!(frames[0][0], FRAME_MAGIC);
        assert_eq!(frames[0][2] as usize, frames.len());
        let rebuilt = frames
            .iter()
            .flat_map(|frame| frame[FRAME_HEADER_SIZE..].iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(rebuilt, payload);
    }

    #[test]
    fn accepts_a_vendor_neutral_identity_contract() {
        let identity = parse_identity(
            br#"{"protocol":"dev.gauge.pairing","version":1,"device_id":"Acme-Desk-A1B2C3","name":"Acme Desk","kind":"display","firmware_version":"2.3.1","capabilities":["display","dashboard.pull"]}"#,
        )
        .unwrap();
        assert_eq!(identity.device_id, "acme-desk-a1b2c3");
        assert_eq!(identity.name.as_deref(), Some("Acme Desk"));
        assert_eq!(identity.kind.as_deref(), Some("display"));
        assert_eq!(identity.firmware_version.as_deref(), Some("2.3.1"));
        assert_eq!(identity.capabilities, ["dashboard.pull", "display"]);
    }

    #[test]
    fn keeps_legacy_version_one_identity_compatible() {
        let identity = parse_identity(
            br#"{"protocol":"dev.gauge.pairing","version":1,"device_id":"legacy-01","capabilities":["display"]}"#,
        )
        .unwrap();
        assert_eq!(identity.device_id, "legacy-01");
        assert_eq!(identity.name, None);
        assert_eq!(identity.kind, None);
    }

    #[test]
    fn rejects_invalid_accessory_metadata() {
        let error = parse_identity(
            br#"{"protocol":"dev.gauge.pairing","version":1,"device_id":"acme-01","name":"Acme","kind":"display","capabilities":["bad capability!"]}"#,
        )
        .unwrap_err();
        assert!(error.contains("capability"));
    }
}
