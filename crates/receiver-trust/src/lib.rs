//! Explicit receiver trust based on the SHA-256 digest of the certificate's
//! SubjectPublicKeyInfo (SPKI). Certificate-chain validation remains the
//! responsibility of the TLS client; this store adds the receiver identity
//! pin required by the FCast protocol.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TrustError {
    #[error("invalid receiver fingerprint '{0}'")]
    InvalidFingerprint(String),
    #[error("unsupported trust file version {version} in {path}")]
    UnsupportedVersion { path: PathBuf, version: u32 },
    #[error("trust store I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("trust store JSON error at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct Fingerprint(String);

impl Fingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Fingerprint {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Fingerprint {
    type Err = TrustError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let encoded = trimmed
            .strip_prefix("sha256/")
            .ok_or_else(|| TrustError::InvalidFingerprint(value.to_owned()))?;
        let bytes = STANDARD
            .decode(encoded)
            .ok()
            .filter(|bytes| bytes.len() == 32)
            .or_else(|| {
                STANDARD_NO_PAD
                    .decode(encoded)
                    .ok()
                    .filter(|bytes| bytes.len() == 32)
            })
            .or_else(|| {
                hex::decode(encoded.replace(':', ""))
                    .ok()
                    .filter(|bytes| bytes.len() == 32)
            })
            .ok_or_else(|| TrustError::InvalidFingerprint(value.to_owned()))?;
        Ok(Self(format!("sha256/{}", STANDARD_NO_PAD.encode(bytes))))
    }
}

impl TryFrom<&str> for Fingerprint {
    type Error = TrustError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<String> for Fingerprint {
    type Error = TrustError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedReceiver {
    pub fingerprint: Fingerprint,
    pub label: Option<String>,
    pub first_seen_unix: u64,
    pub last_seen_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TrustFile {
    version: u32,
    receivers: Vec<TrustedReceiver>,
}

#[derive(Debug, Clone)]
pub struct TrustStore {
    path: PathBuf,
    receivers: BTreeMap<Fingerprint, TrustedReceiver>,
}

impl TrustStore {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, TrustError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                receivers: BTreeMap::new(),
            });
        }
        let contents = fs::read_to_string(&path).map_err(|source| TrustError::Io {
            path: path.clone(),
            source,
        })?;
        let file: TrustFile =
            serde_json::from_str(&contents).map_err(|source| TrustError::Json {
                path: path.clone(),
                source,
            })?;
        if file.version != 1 {
            return Err(TrustError::UnsupportedVersion {
                path,
                version: file.version,
            });
        }
        let receivers = file
            .receivers
            .into_iter()
            .map(|receiver| (receiver.fingerprint.clone(), receiver))
            .collect();
        Ok(Self { path, receivers })
    }

    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::new(),
            receivers: BTreeMap::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> impl Iterator<Item = &TrustedReceiver> {
        self.receivers.values()
    }

    pub fn is_trusted(&self, fingerprint: impl AsRef<str>) -> bool {
        Fingerprint::try_from(fingerprint.as_ref())
            .map(|normalized| self.receivers.contains_key(&normalized))
            .unwrap_or(false)
    }

    pub fn trust(
        &mut self,
        fingerprint: impl AsRef<str>,
        label: Option<String>,
        now_unix: u64,
    ) -> Result<TrustedReceiver, TrustError> {
        let fingerprint = Fingerprint::try_from(fingerprint.as_ref())?;
        let mut receivers = self.receivers.clone();
        let entry = receivers
            .entry(fingerprint.clone())
            .or_insert_with(|| TrustedReceiver {
                fingerprint: fingerprint.clone(),
                label: label.clone(),
                first_seen_unix: now_unix,
                last_seen_unix: now_unix,
            });
        entry.last_seen_unix = now_unix;
        if label.is_some() {
            entry.label = label;
        }
        let result = entry.clone();
        self.persist(&receivers)?;
        self.receivers = receivers;
        Ok(result)
    }

    pub fn forget(&mut self, fingerprint: impl AsRef<str>) -> Result<bool, TrustError> {
        let fingerprint = Fingerprint::try_from(fingerprint.as_ref())?;
        let mut receivers = self.receivers.clone();
        let removed = receivers.remove(&fingerprint).is_some();
        if removed {
            self.persist(&receivers)?;
            self.receivers = receivers;
        }
        Ok(removed)
    }

    pub fn verify_spki(&self, spki_der: &[u8]) -> bool {
        let fingerprint = fingerprint_spki(spki_der);
        self.is_trusted(&fingerprint)
    }

    fn persist(
        &self,
        receivers: &BTreeMap<Fingerprint, TrustedReceiver>,
    ) -> Result<(), TrustError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| TrustError::Io {
                path: parent.to_owned(),
                source,
            })?;
        }
        let body = serde_json::to_vec_pretty(&TrustFile {
            version: 1,
            receivers: receivers.values().cloned().collect(),
        })
        .map_err(|source| TrustError::Json {
            path: self.path.clone(),
            source,
        })?;
        atomic_write_private(&self.path, &body)
    }
}

pub fn fingerprint_spki(spki_der: &[u8]) -> Fingerprint {
    let digest = Sha256::digest(spki_der);
    Fingerprint(format!("sha256/{}", STANDARD_NO_PAD.encode(digest)))
}

pub fn normalize_fingerprint(value: &str) -> Result<String, TrustError> {
    Ok(Fingerprint::try_from(value)?.0)
}

fn atomic_write_private(path: &Path, body: &[u8]) -> Result<(), TrustError> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file: File = options.open(&temporary).map_err(|source| TrustError::Io {
        path: temporary.clone(),
        source,
    })?;
    file.write_all(body).map_err(|source| TrustError::Io {
        path: temporary.clone(),
        source,
    })?;
    file.sync_all().map_err(|source| TrustError::Io {
        path: temporary.clone(),
        source,
    })?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).map_err(|source| {
            TrustError::Io {
                path: temporary.clone(),
                source,
            }
        })?;
    }
    fs::rename(&temporary, path).map_err(|source| TrustError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fcast-trust-test-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn spki_fingerprint_is_canonical_and_round_trips() {
        let fingerprint = fingerprint_spki(b"spki");
        assert_eq!(
            normalize_fingerprint(fingerprint.as_str()).unwrap(),
            fingerprint.as_str()
        );
        assert!(!fingerprint.as_str().contains('='));
    }

    #[test]
    fn fingerprint_serde_round_trip_is_canonical() {
        let canonical = fingerprint_spki(b"serde");
        let padded = format!("{}=", canonical);
        let decoded: Fingerprint = serde_json::from_str(&format!("\"{padded}\"")).unwrap();
        assert_eq!(decoded, canonical);
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            format!("\"{canonical}\"")
        );
    }

    #[test]
    fn rejects_unsupported_trust_file_version() {
        let path = temp_path();
        fs::write(&path, r#"{"version":2,"receivers":[]}"#).unwrap();
        assert!(matches!(
            TrustStore::load(&path),
            Err(TrustError::UnsupportedVersion { version: 2, .. })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_persistence_does_not_change_memory() {
        let blocker = temp_path();
        fs::write(&blocker, b"not a directory").unwrap();
        let fingerprint = fingerprint_spki(b"receiver-key");
        let mut store = TrustStore::in_memory();
        store.trust(&fingerprint, None, 1).unwrap();
        store.path = blocker.join("trust.json");

        assert!(
            store
                .trust(fingerprint_spki(b"other-key"), None, 2)
                .is_err()
        );
        assert_eq!(store.list().count(), 1);
        assert!(store.forget(&fingerprint).is_err());
        assert!(store.is_trusted(&fingerprint));
        fs::remove_file(blocker).unwrap();
    }

    #[test]
    fn trust_store_persists_private_entries() {
        let path = temp_path();
        let mut store = TrustStore::load(&path).unwrap();
        let fingerprint = fingerprint_spki(b"receiver-key");
        store
            .trust(&fingerprint, Some("Living room".into()), 42)
            .unwrap();
        assert!(TrustStore::load(&path).unwrap().is_trusted(&fingerprint));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(path).unwrap();
    }
}
