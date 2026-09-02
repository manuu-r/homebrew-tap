//! Paired accessory identities and credentials.
//!
//! Device metadata is readable JSON so it can be migrated and inspected. The
//! bearer credentials themselves live in the user's macOS Keychain and are
//! cached only in this process after Gauge starts.

use getrandom::fill as fill_random;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use subtle::ConstantTimeEq;

use crate::{config, now_seconds};

pub const ACCESSORY_PROTOCOL: &str = "dev.gauge.accessory";
pub const ACCESSORY_API_VERSION: u16 = 1;
pub const DASHBOARD_PATH: &str = "/v1/dashboard";
pub const SERVICE_TYPE: &str = "_gauge._tcp.local.";
const REGISTRY_FILE: &str = "devices.json";
const KEYCHAIN_SERVICE: &str = "dev.gauge.accessory";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    pub protocol_version: u16,
    pub capabilities: Vec<String>,
    pub paired_at: u64,
    pub last_seen_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryFile {
    schema_version: u16,
    #[serde(default)]
    server_id: String,
    devices: Vec<PairedDevice>,
}

impl Default for RegistryFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            server_id: String::new(),
            devices: Vec::new(),
        }
    }
}

/// App-specific configuration sent through the MITM-protected BLE
/// commissioning characteristic alongside the Wi-Fi credentials.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PairingBundle {
    pub protocol: &'static str,
    pub version: u16,
    pub device_id: String,
    pub server_id: String,
    pub bearer_token: String,
    pub service_type: &'static str,
    pub dashboard_path: &'static str,
    pub server_port: u16,
}

trait CredentialVault: Send + Sync {
    fn set(&self, device_id: &str, secret: &str) -> Result<(), String>;
    fn get(&self, device_id: &str) -> Result<Option<String>, String>;
    fn delete(&self, device_id: &str) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
struct KeychainVault;

#[cfg(target_os = "macos")]
impl CredentialVault for KeychainVault {
    fn set(&self, device_id: &str, secret: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(
            KEYCHAIN_SERVICE,
            &keychain_account(device_id),
            secret.as_bytes(),
        )
        .map_err(|error| format!("could not save the device credential in Keychain: {error}"))
    }

    fn get(&self, device_id: &str) -> Result<Option<String>, String> {
        use security_framework::passwords::{generic_password, PasswordOptions};
        use security_framework_sys::base::errSecItemNotFound;

        let options =
            PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, &keychain_account(device_id));
        match generic_password(options) {
            Ok(secret) => String::from_utf8(secret)
                .map(Some)
                .map_err(|_| "the device credential in Keychain is not valid UTF-8".into()),
            Err(error) if error.code() == errSecItemNotFound => Ok(None),
            Err(error) => Err(format!(
                "could not read the device credential from Keychain: {error}"
            )),
        }
    }

    fn delete(&self, device_id: &str) -> Result<(), String> {
        use security_framework::passwords::delete_generic_password;
        use security_framework_sys::base::errSecItemNotFound;

        match delete_generic_password(KEYCHAIN_SERVICE, &keychain_account(device_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == errSecItemNotFound => Ok(()),
            Err(error) => Err(format!(
                "could not remove the device credential from Keychain: {error}"
            )),
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct KeychainVault;

#[cfg(not(target_os = "macos"))]
impl CredentialVault for KeychainVault {
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("paired accessories require the macOS Keychain".into())
    }

    fn get(&self, _: &str) -> Result<Option<String>, String> {
        Err("paired accessories require the macOS Keychain".into())
    }

    fn delete(&self, _: &str) -> Result<(), String> {
        Err("paired accessories require the macOS Keychain".into())
    }
}

fn keychain_account(device_id: &str) -> String {
    format!("device:{device_id}")
}

/// Thread-safe device registry shared by the tray, provisioning controller,
/// and background HTTP service.
pub struct DeviceStore {
    path: PathBuf,
    registry: RwLock<RegistryFile>,
    secrets: RwLock<HashMap<String, String>>,
    vault: Arc<dyn CredentialVault>,
}

impl DeviceStore {
    pub fn open() -> Result<Self, String> {
        let directory = config::path()?
            .parent()
            .ok_or("settings path has no containing directory")?
            .to_path_buf();
        Self::open_with(directory.join(REGISTRY_FILE), Arc::new(KeychainVault))
    }

    fn open_with(path: PathBuf, vault: Arc<dyn CredentialVault>) -> Result<Self, String> {
        let mut registry = read_registry(&path)?;
        if registry.server_id.is_empty() {
            registry.server_id = random_hex::<16>()?;
            save_registry(&path, &registry)?;
        }
        let mut secrets = HashMap::new();
        for device in &registry.devices {
            if let Some(secret) = vault.get(&device.id)? {
                secrets.insert(device.id.clone(), secret);
            }
        }
        Ok(Self {
            path,
            registry: RwLock::new(registry),
            secrets: RwLock::new(secrets),
            vault,
        })
    }

    pub fn devices(&self) -> Vec<PairedDevice> {
        self.registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .devices
            .clone()
    }

    pub fn server_id(&self) -> String {
        self.registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .server_id
            .clone()
    }

    /// Create or replace one device's credential. The caller passes the stable
    /// hardware identity read after BLE Secure Connections authentication.
    pub fn pair(
        &self,
        device_id: &str,
        name: &str,
        kind: &str,
        firmware_version: Option<&str>,
        capabilities: Vec<String>,
        server_port: u16,
    ) -> Result<PairingBundle, String> {
        validate_identifier(device_id)?;
        let name = clean_label(name, "device name")?;
        let kind = clean_label(kind, "device kind")?;
        let firmware_version = firmware_version
            .map(|version| clean_label(version, "firmware version"))
            .transpose()?;
        if server_port == 0 {
            return Err("accessory server port must not be zero".into());
        }

        let secret = random_hex::<32>()?;
        self.vault.set(device_id, &secret)?;

        let now = now_seconds();
        let device = PairedDevice {
            id: device_id.into(),
            name,
            kind,
            firmware_version,
            protocol_version: ACCESSORY_API_VERSION,
            capabilities: normalize_capabilities(capabilities),
            paired_at: now,
            last_seen_at: None,
        };

        let save_result = {
            let mut registry = self
                .registry
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            registry.devices.retain(|existing| existing.id != device_id);
            registry.devices.push(device);
            registry
                .devices
                .sort_by(|left, right| left.name.cmp(&right.name));
            save_registry(&self.path, &registry)
        };
        if let Err(error) = save_result {
            let _ = self.vault.delete(device_id);
            return Err(error);
        }

        self.secrets
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(device_id.into(), secret.clone());

        Ok(PairingBundle {
            protocol: ACCESSORY_PROTOCOL,
            version: ACCESSORY_API_VERSION,
            device_id: device_id.into(),
            server_id: self.server_id(),
            bearer_token: secret,
            service_type: SERVICE_TYPE,
            dashboard_path: DASHBOARD_PATH,
            server_port,
        })
    }

    pub fn authenticate(&self, supplied: &str) -> Option<String> {
        if supplied.len() != 64 || !supplied.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let secrets = self
            .secrets
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        secrets.iter().find_map(|(device_id, expected)| {
            (expected.len() == supplied.len()
                && bool::from(expected.as_bytes().ct_eq(supplied.as_bytes())))
            .then(|| device_id.clone())
        })
    }

    pub fn revoke(&self, device_id: &str) -> Result<bool, String> {
        let removed = {
            let mut registry = self
                .registry
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let before = registry.devices.len();
            registry.devices.retain(|device| device.id != device_id);
            let removed = registry.devices.len() != before;
            if removed {
                save_registry(&self.path, &registry)?;
            }
            removed
        };
        if removed {
            self.secrets
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(device_id);
            self.vault.delete(device_id)?;
        }
        Ok(removed)
    }
}

fn read_registry(path: &Path) -> Result<RegistryFile, String> {
    match fs::read_to_string(path) {
        Ok(body) => {
            let registry: RegistryFile = serde_json::from_str(&body).map_err(|error| {
                format!("invalid device registry at {}: {error}", path.display())
            })?;
            if registry.schema_version != 1 {
                return Err(format!(
                    "unsupported device registry schema {} at {}",
                    registry.schema_version,
                    path.display()
                ));
            }
            Ok(registry)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RegistryFile::default()),
        Err(error) => Err(format!(
            "could not read device registry at {}: {error}",
            path.display()
        )),
    }
}

fn save_registry(path: &Path, registry: &RegistryFile) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or("device registry path has no containing directory")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create device registry directory: {error}"))?;
    let body = serde_json::to_string_pretty(registry)
        .map_err(|error| format!("could not encode device registry: {error}"))?;
    fs::write(path, format!("{body}\n")).map_err(|error| {
        format!(
            "could not write device registry at {}: {error}",
            path.display()
        )
    })
}

fn validate_identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("device id must be 1-64 ASCII letters, digits, '.', '-', or '_'".into());
    }
    Ok(())
}

fn clean_label(value: &str, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 80 || value.chars().any(char::is_control) {
        return Err(format!("{field} must be 1-80 printable characters"));
    }
    Ok(value.into())
}

fn normalize_capabilities(capabilities: Vec<String>) -> Vec<String> {
    let mut capabilities = capabilities
        .into_iter()
        .map(|capability| capability.trim().to_ascii_lowercase())
        .filter(|capability| {
            !capability.is_empty()
                && capability.len() <= 48
                && capability.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'-' | b'_')
                })
        })
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn random_hex<const N: usize>() -> Result<String, String> {
    let mut bytes = [0_u8; N];
    fill_random(&mut bytes).map_err(|error| format!("secure random generation failed: {error}"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(N * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryVault(Mutex<HashMap<String, String>>);

    impl CredentialVault for MemoryVault {
        fn set(&self, device_id: &str, secret: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .insert(device_id.into(), secret.into());
            Ok(())
        }

        fn get(&self, device_id: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(device_id).cloned())
        }

        fn delete(&self, device_id: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(device_id);
            Ok(())
        }
    }

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gauge-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn pairs_authenticates_and_revokes_independent_devices() {
        let path = test_path("devices");
        let _ = fs::remove_file(&path);
        let store = DeviceStore::open_with(path.clone(), Arc::new(MemoryVault::default())).unwrap();
        let first = store
            .pair(
                "acme-a1",
                "Acme display",
                "display",
                Some("0.1.0"),
                vec!["dashboard.pull".into(), "audio.output".into()],
                45_831,
            )
            .unwrap();
        let second = store
            .pair(
                "desk-b2",
                "Desk display",
                "display",
                None,
                vec!["dashboard.pull".into()],
                45_831,
            )
            .unwrap();

        assert_ne!(first.bearer_token, second.bearer_token);
        let paired = store.devices();
        assert_eq!(paired[0].firmware_version.as_deref(), Some("0.1.0"));
        assert_eq!(paired[1].firmware_version, None);
        assert_eq!(
            store.authenticate(&first.bearer_token).as_deref(),
            Some("acme-a1")
        );
        assert_eq!(
            store.authenticate(&second.bearer_token).as_deref(),
            Some("desk-b2")
        );
        assert!(store.revoke("acme-a1").unwrap());
        assert!(store.authenticate(&first.bearer_token).is_none());
        assert_eq!(
            store.authenticate(&second.bearer_token).as_deref(),
            Some("desk-b2")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn validates_device_identity_and_normalizes_capabilities() {
        assert!(validate_identifier("acme-01").is_ok());
        assert!(validate_identifier("bad id").is_err());
        assert_eq!(
            normalize_capabilities(vec![
                " Dashboard.Pull ".into(),
                "dashboard.pull".into(),
                "bad capability!".into(),
            ]),
            ["dashboard.pull"]
        );
    }

    #[test]
    fn reads_registry_entries_written_before_firmware_metadata() {
        let registry: RegistryFile = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "server_id": "server-01",
                "devices": [{
                    "id": "legacy-01",
                    "name": "Legacy display",
                    "kind": "display",
                    "protocol_version": 1,
                    "capabilities": ["display"],
                    "paired_at": 1,
                    "last_seen_at": null
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(registry.devices[0].firmware_version, None);
    }
}
