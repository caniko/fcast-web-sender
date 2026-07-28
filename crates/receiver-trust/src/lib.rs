//! Explicit receiver trust based on the SHA-256 digest of the certificate's
//! SubjectPublicKeyInfo (SPKI). Certificate-chain validation remains the
//! responsibility of the TLS client; this store adds the receiver identity
//! pin required by the FCast protocol.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrustError {
    #[error("invalid receiver fingerprint '{0}'")]
    InvalidFingerprint(String),
    #[error("trust store I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("trust store JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedReceiver {
    pub fingerprint: String,
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
    receivers: BTreeMap<String, TrustedReceiver>,
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
        let contents = fs::read_to_string(&path)?;
        let file: TrustFile = serde_json::from_str(&contents)?;
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

    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        normalize_fingerprint(fingerprint)
            .map(|normalized| self.receivers.contains_key(&normalized))
            .unwrap_or(false)
    }

    pub fn trust(
        &mut self,
        fingerprint: &str,
        label: Option<String>,
        now_unix: u64,
    ) -> Result<TrustedReceiver, TrustError> {
        let fingerprint = normalize_fingerprint(fingerprint)?;
        let entry = self
            .receivers
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
        self.persist()?;
        Ok(result)
    }

    pub fn forget(&mut self, fingerprint: &str) -> Result<bool, TrustError> {
        let fingerprint = normalize_fingerprint(fingerprint)?;
        let removed = self.receivers.remove(&fingerprint).is_some();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn verify_spki(&self, spki_der: &[u8]) -> bool {
        let fingerprint = fingerprint_spki(spki_der);
        self.is_trusted(&fingerprint)
    }

    fn persist(&self) -> Result<(), TrustError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_vec_pretty(&TrustFile {
            version: 1,
            receivers: self.receivers.values().cloned().collect(),
        })?;
        atomic_write_private(&self.path, &body)
    }
}

pub fn fingerprint_spki(spki_der: &[u8]) -> String {
    let digest = Sha256::digest(spki_der);
    format!("sha256/{}", STANDARD_NO_PAD.encode(digest))
}

pub fn normalize_fingerprint(value: &str) -> Result<String, TrustError> {
    let trimmed = value.trim();
    let encoded = trimmed
        .strip_prefix("sha256/")
        .ok_or_else(|| TrustError::InvalidFingerprint(value.to_owned()))?;
    let bytes = STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .or_else(|_| hex::decode(encoded.replace(':', "")))
        .map_err(|_| TrustError::InvalidFingerprint(value.to_owned()))?;
    if bytes.len() != 32 {
        return Err(TrustError::InvalidFingerprint(value.to_owned()));
    }
    Ok(format!("sha256/{}", STANDARD_NO_PAD.encode(bytes)))
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
    let mut file: File = options.open(&temporary)?;
    file.write_all(body)?;
    file.sync_all()?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
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
        assert_eq!(normalize_fingerprint(&fingerprint).unwrap(), fingerprint);
        assert!(!fingerprint.contains('='));
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
