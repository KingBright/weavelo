use rand::random;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

pub const MESH_CONFIG_FILE_NAME: &str = "mesh.conf";
pub const MESH_SECRET_FILE_NAME: &str = "mesh.secret";
pub const WEAVELO_DEVICE_GROUP_DIR_ENV: &str = "WEAVELO_DEVICE_GROUP_DIR";
pub const WEAVELO_DEVICE_GROUP_COPY_CODE_PREFIX: &str = "RPM2";

#[derive(Clone, PartialEq, Eq)]
pub struct MeshSecret(String);

impl MeshSecret {
    pub fn new(value: impl Into<String>) -> Result<Self, MeshConfigError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(MeshConfigError::EmptyNetworkSecret);
        }
        Ok(Self(value))
    }

    pub fn generate() -> Self {
        Self(hex_random::<32>())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MeshSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MeshSecret(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshConfig {
    pub network_name: String,
    pub network_secret: MeshSecret,
    pub node_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PublicMeshIdentity {
    pub network_name_sha256: String,
    pub device_id_sha256: String,
}

/// Stable comparison of public metadata; callers must not pass credentials/capabilities.
pub fn public_metadata_fingerprint(value: &str) -> String {
    format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(value.as_bytes()))
}

impl MeshConfig {
    /// Hash public identity metadata only; never export invitation or secret material.
    pub fn public_identity(&self) -> PublicMeshIdentity {
        PublicMeshIdentity {
            network_name_sha256: public_metadata_fingerprint(&self.network_name),
            device_id_sha256: public_metadata_fingerprint(&self.node_id),
        }
    }
    pub fn generate(display_name: impl Into<String>) -> Self {
        Self {
            network_name: format!("weavelo-{}", hex_random::<8>()),
            network_secret: MeshSecret::generate(),
            node_id: hex_random::<16>(),
            display_name: sanitized_display_name(display_name),
        }
    }

    pub fn from_invite_code(
        invite_code: &str,
        display_name: impl Into<String>,
    ) -> Result<Self, MeshConfigError> {
        let invite = MeshInvite::decode(invite_code)?;
        Ok(Self {
            network_name: invite.network_name,
            network_secret: invite.network_secret,
            node_id: hex_random::<16>(),
            display_name: sanitized_display_name(display_name),
        })
    }

    pub fn invite_code(&self) -> String {
        MeshInvite {
            network_name: self.network_name.clone(),
            network_secret: self.network_secret.clone(),
        }
        .encode_copy_code()
    }

    pub fn validate(&self) -> Result<(), MeshConfigError> {
        if self.network_name.trim().is_empty() {
            return Err(MeshConfigError::EmptyNetworkName);
        }
        if self.network_secret.expose_secret().trim().is_empty() {
            return Err(MeshConfigError::EmptyNetworkSecret);
        }
        if self.node_id.trim().is_empty() {
            return Err(MeshConfigError::EmptyNodeId);
        }
        if self.display_name.trim().is_empty() {
            return Err(MeshConfigError::EmptyDisplayName);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshInvite {
    pub network_name: String,
    pub network_secret: MeshSecret,
}

impl MeshInvite {
    pub fn encode(&self) -> String {
        self.encode_copy_code()
    }

    pub fn decode(value: &str) -> Result<Self, MeshConfigError> {
        decode_mesh_copy_code(value)
    }

    pub fn encode_copy_code(&self) -> String {
        let mut payload = Vec::new();
        payload.push(MESH_COPY_CODE_VERSION);
        push_u8_len_bytes(&mut payload, self.network_name.as_bytes());
        let (secret_kind, secret_bytes) =
            encode_copy_code_secret(self.network_secret.expose_secret());
        payload.push(secret_kind);
        push_u8_len_bytes(&mut payload, &secret_bytes);
        let checksum = fnv1a32(&payload);
        payload.extend_from_slice(&checksum.to_be_bytes());
        let encoded = crockford_base32_encode(&payload);
        format!(
            "{WEAVELO_DEVICE_GROUP_COPY_CODE_PREFIX}-{}",
            group_copy_code(&encoded)
        )
    }
}

const MESH_COPY_CODE_VERSION: u8 = 2;
const MESH_COPY_SECRET_UTF8: u8 = 0;
const MESH_COPY_SECRET_HEX_BYTES: u8 = 1;

fn decode_mesh_copy_code(value: &str) -> Result<MeshInvite, MeshConfigError> {
    let compact = compact_copy_code(value);
    if !compact
        .get(..WEAVELO_DEVICE_GROUP_COPY_CODE_PREFIX.len())
        .is_some_and(|prefix| {
            prefix.eq_ignore_ascii_case(WEAVELO_DEVICE_GROUP_COPY_CODE_PREFIX)
        })
    {
        return Err(MeshConfigError::InvalidInviteCode);
    }
    let encoded = &compact[WEAVELO_DEVICE_GROUP_COPY_CODE_PREFIX.len()..];
    let payload = crockford_base32_decode(encoded)?;
    if payload.len() < 8 {
        return Err(MeshConfigError::InvalidInviteCode);
    }
    let content_len = payload.len() - 4;
    let (content, checksum_bytes) = payload.split_at(content_len);
    let expected = u32::from_be_bytes(
        checksum_bytes
            .try_into()
            .map_err(|_| MeshConfigError::InvalidInviteCode)?,
    );
    if fnv1a32(content) != expected {
        return Err(MeshConfigError::InvalidInviteCode);
    }

    let mut reader = MeshCopyCodeReader::new(content);
    if reader.read_u8()? != MESH_COPY_CODE_VERSION {
        return Err(MeshConfigError::InvalidInviteCode);
    }
    let network_name = String::from_utf8(reader.read_u8_len_bytes()?.to_vec())
        .map_err(|_| MeshConfigError::InvalidInviteCode)?;
    if network_name.trim().is_empty() {
        return Err(MeshConfigError::EmptyNetworkName);
    }
    let secret_kind = reader.read_u8()?;
    let secret = decode_copy_code_secret(secret_kind, reader.read_u8_len_bytes()?)?;
    if !reader.is_finished() {
        return Err(MeshConfigError::InvalidInviteCode);
    }
    Ok(MeshInvite {
        network_name,
        network_secret: MeshSecret::new(secret)?,
    })
}

fn encode_copy_code_secret(secret: &str) -> (u8, Vec<u8>) {
    if let Some(bytes) = decode_hex_bytes_for_copy_code(secret) {
        (MESH_COPY_SECRET_HEX_BYTES, bytes)
    } else {
        (MESH_COPY_SECRET_UTF8, secret.as_bytes().to_vec())
    }
}

fn decode_copy_code_secret(secret_kind: u8, bytes: &[u8]) -> Result<String, MeshConfigError> {
    match secret_kind {
        MESH_COPY_SECRET_UTF8 => {
            String::from_utf8(bytes.to_vec()).map_err(|_| MeshConfigError::InvalidInviteCode)
        }
        MESH_COPY_SECRET_HEX_BYTES => Ok(hex_bytes(bytes)),
        _ => Err(MeshConfigError::InvalidInviteCode),
    }
}

fn push_u8_len_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(bytes.len().try_into().unwrap_or(u8::MAX));
    out.extend_from_slice(&bytes[..bytes.len().min(u8::MAX as usize)]);
}

fn compact_copy_code(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect()
}

fn group_copy_code(encoded: &str) -> String {
    const GROUP: usize = 8;
    let mut grouped = String::with_capacity(encoded.len() + encoded.len() / GROUP);
    for (idx, ch) in encoded.chars().enumerate() {
        if idx > 0 && idx % GROUP == 0 {
            grouped.push('-');
        }
        grouped.push(ch);
    }
    grouped
}

fn crockford_base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut out = String::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            let shift = bits - 5;
            out.push(ALPHABET[((buffer >> shift) & 0x1f) as usize] as char);
            bits -= 5;
            buffer &= (1 << bits) - 1;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn crockford_base32_decode(value: &str) -> Result<Vec<u8>, MeshConfigError> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for ch in value.chars() {
        let value = crockford_base32_value(ch).ok_or(MeshConfigError::InvalidInviteCode)?;
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        while bits >= 8 {
            let shift = bits - 8;
            out.push(((buffer >> shift) & 0xff) as u8);
            bits -= 8;
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn crockford_base32_value(ch: char) -> Option<u8> {
    match ch.to_ascii_uppercase() {
        '0' | 'O' => Some(0),
        '1' | 'I' | 'L' => Some(1),
        '2' => Some(2),
        '3' => Some(3),
        '4' => Some(4),
        '5' => Some(5),
        '6' => Some(6),
        '7' => Some(7),
        '8' => Some(8),
        '9' => Some(9),
        'A' => Some(10),
        'B' => Some(11),
        'C' => Some(12),
        'D' => Some(13),
        'E' => Some(14),
        'F' => Some(15),
        'G' => Some(16),
        'H' => Some(17),
        'J' => Some(18),
        'K' => Some(19),
        'M' => Some(20),
        'N' => Some(21),
        'P' => Some(22),
        'Q' => Some(23),
        'R' => Some(24),
        'S' => Some(25),
        'T' => Some(26),
        'V' => Some(27),
        'W' => Some(28),
        'X' => Some(29),
        'Y' => Some(30),
        'Z' => Some(31),
        _ => None,
    }
}

fn decode_hex_bytes_for_copy_code(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let raw = value.as_bytes();
    for idx in (0..raw.len()).step_by(2) {
        let high = decode_hex_nibble_for_copy_code(raw[idx])?;
        let low = decode_hex_nibble_for_copy_code(raw[idx + 1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

fn decode_hex_nibble_for_copy_code(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

struct MeshCopyCodeReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> MeshCopyCodeReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn read_u8(&mut self) -> Result<u8, MeshConfigError> {
        let value = *self
            .bytes
            .get(self.pos)
            .ok_or(MeshConfigError::InvalidInviteCode)?;
        self.pos += 1;
        Ok(value)
    }
    fn read_u8_len_bytes(&mut self) -> Result<&'a [u8], MeshConfigError> {
        let len = self.read_u8()? as usize;
        let end = self
            .pos
            .checked_add(len)
            .ok_or(MeshConfigError::InvalidInviteCode)?;
        if end > self.bytes.len() {
            return Err(MeshConfigError::InvalidInviteCode);
        }
        let result = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(result)
    }
    fn is_finished(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshSecretStorageKind {
    AppPrivateFile,
    PlatformSecureStore,
}

/// Recovery data has exactly the credential backend's protection, never a public
/// journal or receipt. Debug deliberately redacts metadata as well as secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct MeshRecoveryRecord {
    profile: String,
    before: StoredMeshPair,
    after: StoredMeshPair,
}
impl fmt::Debug for MeshRecoveryRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MeshRecoveryRecord(<redacted>)")
    }
}
#[derive(Clone, PartialEq, Eq)]
struct StoredMeshPair {
    metadata: Option<String>,
    secret: Option<MeshSecret>,
}
impl StoredMeshPair {
    fn config(&self) -> Result<Option<MeshConfig>, MeshStoreError> {
        match (&self.metadata, &self.secret) {
            (None, None) => Ok(None),
            (Some(metadata), Some(secret)) => {
                let mut config = decode_mesh_config_metadata(metadata)?;
                config.network_secret = secret.clone();
                config.validate().map_err(MeshStoreError::InvalidMeshConfig)?;
                Ok(Some(config))
            }
            (Some(_), None) => Err(MeshStoreError::MissingNetworkSecret),
            (None, Some(_)) => Err(transaction_error("configuration is missing but its existing secret is retained; explicit recovery is required")),
        }
    }
}
fn transaction_error(message: &'static str) -> MeshStoreError {
    MeshStoreError::InvalidMetadata(message.into())
}

pub trait MeshSecretStore {
    fn storage_kind(&self) -> MeshSecretStorageKind;
    fn load_secret(&self) -> Result<Option<MeshSecret>, MeshStoreError>;
    fn save_secret(&self, secret: &MeshSecret) -> Result<(), MeshStoreError>;
    fn delete_secret(&self) -> Result<(), MeshStoreError>;
    /// Implementations must keep recovery in the SAME protected credential
    /// backend. Unsupported backends fail before either canonical value changes.
    fn load_recovery(&self) -> Result<Option<MeshRecoveryRecord>, MeshStoreError> { Ok(None) }
    fn save_recovery(&self, _record: &MeshRecoveryRecord) -> Result<(), MeshStoreError> {
        Err(transaction_error("credential backend does not support protected recovery"))
    }
    fn delete_recovery(&self) -> Result<(), MeshStoreError> {
        Err(transaction_error("credential backend does not support protected recovery"))
    }
    /// Check access without loading credentials or changing their protection.
    fn validate_access(&self) -> Result<(), MeshStoreError> {
        Ok(())
    }
}

pub fn default_app_private_mesh_dir() -> PathBuf {
    if let Some(path) = env::var_os(WEAVELO_DEVICE_GROUP_DIR_ENV)
        .and_then(non_empty_env_path)
        .map(PathBuf::from)
    {
        return path;
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME").and_then(non_empty_env_path) {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Weavelo")
            .join(&crate::desktop_identity::identity().profile_directory);
    }
    #[cfg(target_os = "windows")]
    if let Some(appdata) = env::var_os("APPDATA").and_then(non_empty_env_path) {
        return PathBuf::from(appdata).join("Weavelo").join(&crate::desktop_identity::identity().profile_directory);
    }
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let profile_folder = "desktop";
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let profile_folder = "mesh";
    #[cfg(not(target_os = "windows"))]
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME").and_then(non_empty_env_path) {
        return PathBuf::from(config_home).join("weavelo").join(profile_folder);
    }
    #[cfg(not(target_os = "windows"))]
    if let Some(home) = env::var_os("HOME").and_then(non_empty_env_path) {
        return PathBuf::from(home)
            .join(".config")
            .join("weavelo")
            .join(profile_folder);
    }
    env::temp_dir().join("weavelo").join(profile_folder)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPrivateMeshSecretStore {
    path: PathBuf,
}
impl AppPrivateMeshSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn recovery_path(&self) -> PathBuf { self.path.with_extension("secret-recovery") }

}
impl MeshSecretStore for AppPrivateMeshSecretStore {
    fn storage_kind(&self) -> MeshSecretStorageKind {
        MeshSecretStorageKind::AppPrivateFile
    }
    fn load_secret(&self) -> Result<Option<MeshSecret>, MeshStoreError> {
        match read_private_file(&self.path)? {
            Some(value) => MeshSecret::new(value.trim())
                .map(Some)
                .map_err(MeshStoreError::InvalidMeshConfig),
            None => Ok(None),
        }
    }
    fn save_secret(&self, secret: &MeshSecret) -> Result<(), MeshStoreError> {
        write_private_file(&self.path, secret.expose_secret().as_bytes())
    }
    fn delete_secret(&self) -> Result<(), MeshStoreError> {
        remove_file_if_exists(&self.path)
    }
    fn load_recovery(&self) -> Result<Option<MeshRecoveryRecord>, MeshStoreError> {
        read_private_file(&self.recovery_path())?.map(|value| decode_recovery(&value)).transpose()
    }
    fn save_recovery(&self, record: &MeshRecoveryRecord) -> Result<(), MeshStoreError> {
        if self.load_recovery()?.is_some() {
            return Err(transaction_error("pending recovery retained; mutation refused"));
        }
        write_private_file(&self.recovery_path(), encode_recovery(record)?.as_bytes())
    }
    fn delete_recovery(&self) -> Result<(), MeshStoreError> {
        remove_file_if_exists(&self.recovery_path())
    }
    fn validate_access(&self) -> Result<(), MeshStoreError> {
        validate_private_parent(&self.path)?;
        private_file_metadata(&self.recovery_path())?;
        private_file_metadata(&self.path).map(|_| ())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryWire {
    version: u32,
    profile: String,
    before_metadata: Option<String>,
    before_secret: Option<String>,
    after_metadata: Option<String>,
    after_secret: Option<String>,
}
const PRIVATE_ENTRY_LIMIT: u64 = 1024 * 1024;
fn encode_recovery(record: &MeshRecoveryRecord) -> Result<String, MeshStoreError> {
    let encoded = serde_json::to_string(&RecoveryWire {
        version: 1, profile: record.profile.clone(),
        before_metadata: record.before.metadata.clone(),
        before_secret: record.before.secret.as_ref().map(|s| s.expose_secret().to_owned()),
        after_metadata: record.after.metadata.clone(),
        after_secret: record.after.secret.as_ref().map(|s| s.expose_secret().to_owned()),
    }).map_err(|_| transaction_error("could not encode protected recovery"))?;
    if encoded.len() as u64 > PRIVATE_ENTRY_LIMIT {
        return Err(transaction_error("protected recovery exceeds size limit; original configuration retained"));
    }
    if decode_recovery(&encoded)?.ne(record) {
        return Err(transaction_error("protected recovery encoding did not verify"));
    }
    Ok(encoded)
}
fn decode_recovery(value: &str) -> Result<MeshRecoveryRecord, MeshStoreError> {
    let wire: RecoveryWire = serde_json::from_str(value)
        .map_err(|_| transaction_error("invalid protected recovery; retained without replay"))?;
    if wire.version != 1 { return Err(transaction_error("unknown protected recovery version")); }
    let record = MeshRecoveryRecord {
        profile: wire.profile,
        before: StoredMeshPair { metadata: wire.before_metadata,
            secret: wire.before_secret.map(MeshSecret::new).transpose().map_err(MeshStoreError::InvalidMeshConfig)? },
        after: StoredMeshPair { metadata: wire.after_metadata,
            secret: wire.after_secret.map(MeshSecret::new).transpose().map_err(MeshStoreError::InvalidMeshConfig)? },
    };
    record.before.config()?;
    record.after.config()?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPrivateMeshConfigStore<S = AppPrivateMeshSecretStore> {
    root_dir: PathBuf,
    config_path: PathBuf,
    secret_store: S,
}
struct MeshProfileLock(fs::File);
impl Drop for MeshProfileLock {
    fn drop(&mut self) {
        // A concurrent fork can briefly retain the same open file description
        // until exec closes CLOEXEC descriptors. Closing only this owner's fd
        // would leave its flock held after the transaction scope has ended.
        // Unlock while ownership is still explicit, then close the descriptor.
        // Failure still falls back to close; no retry or concurrency bypass.
        let _ = self.0.unlock();
    }
}
impl AppPrivateMeshConfigStore<AppPrivateMeshSecretStore> {
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        let root_dir = root_dir.into();
        let config_path = root_dir.join(MESH_CONFIG_FILE_NAME);
        let secret_store = AppPrivateMeshSecretStore::new(root_dir.join(MESH_SECRET_FILE_NAME));
        Self {
            root_dir,
            config_path,
            secret_store,
        }
    }
}
impl<S: MeshSecretStore> AppPrivateMeshConfigStore<S> {
    pub fn with_secret_store(root_dir: impl Into<PathBuf>, secret_store: S) -> Self {
        let root_dir = root_dir.into();
        let config_path = root_dir.join(MESH_CONFIG_FILE_NAME);
        Self {
            root_dir,
            config_path,
            secret_store,
        }
    }
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
    pub fn secret_store(&self) -> &S {
        &self.secret_store
    }
    fn validate_access(&self) -> Result<(), MeshStoreError> {
        validate_private_dir(&self.root_dir)?;
        private_file_metadata(&self.config_path)?;
        self.secret_store.validate_access()
    }
    fn lock(&self) -> Result<MeshProfileLock, MeshStoreError> {
        self.validate_access()?;
        create_private_dir(&self.root_dir)?;
        #[cfg(unix)]
        {
            // Directory locks survive atomic replacement of either profile
            // file and work for an existing read-only legacy profile without
            // creating an entry or widening access.
            let path = &self.root_dir;
            let before = fs::symlink_metadata(path).map_err(|e| MeshStoreError::io(path.clone(), e))?;
            let mut options = fs::OpenOptions::new();
            options.read(true).custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_NONBLOCK);
            let file = options.open(path).map_err(|e| MeshStoreError::io(path.clone(), e))?;
            let opened = file.metadata().map_err(|e| MeshStoreError::io(path.clone(), e))?;
            if !same_profile_file(&before, &opened) { return Err(transaction_error("profile changed while locking")); }
            file.try_lock().map_err(|_| transaction_error("profile is busy; mutation/read refused"))?;
            let lock = MeshProfileLock(file);
            validate_private_dir(path)?;
            let now = fs::symlink_metadata(path).map_err(|e| MeshStoreError::io(path.clone(), e))?;
            if !same_profile_file(&now, &opened) { return Err(transaction_error("profile lock changed; nothing replayed")); }
            Ok(lock)
        }
        #[cfg(not(unix))]
        {
            let path = self.root_dir.join(".mesh-config.lock");
            private_file_metadata(&path)?;
            let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
                .open(&path).map_err(|e| MeshStoreError::io(path.clone(), e))?;
            let opened = file.metadata().map_err(|e| MeshStoreError::io(path.clone(), e))?;
            check_private_file(&path, &opened)?;
            file.try_lock().map_err(|_| transaction_error("profile is busy; mutation/read refused"))?;
            let lock = MeshProfileLock(file);
            if private_file_metadata(&path)?.as_ref().is_none_or(|now| !same_profile_file(now, &opened)) {
                return Err(transaction_error("profile lock changed; nothing replayed"));
            }
            Ok(lock)
        }
    }

    fn profile_binding(&self) -> Result<String, MeshStoreError> {
        self.root_dir.canonicalize().map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| MeshStoreError::io(self.root_dir.clone(), e))
    }
    fn pair(&self) -> Result<StoredMeshPair, MeshStoreError> {
        Ok(StoredMeshPair { metadata: read_private_file(&self.config_path)?,
            secret: self.secret_store.load_secret()? })
    }
    fn refuse_pending(&self) -> Result<(), MeshStoreError> {
        if self.secret_store.load_recovery()?.is_some() {
            return Err(transaction_error("pending recovery retained; read/recover original state before a new mutation"));
        }
        Ok(())
    }
    /// Only a known before/after tuple may be rolled BACK. Never replay forward.
    fn recover_locked(&self) -> Result<bool, MeshStoreError> {
        let Some(record) = self.secret_store.load_recovery()? else { return Ok(false); };
        record.before.config()?;
        record.after.config()?;
        let current = self.pair()?;
        if record.profile != self.profile_binding()?
            || (current.metadata != record.before.metadata && current.metadata != record.after.metadata)
            || (current.secret != record.before.secret && current.secret != record.after.secret) {
            return Err(transaction_error("recovery does not match profile/current tuple; retained without replay"));
        }
        self.write_pair(&record.before)?;
        if self.pair()? != record.before { return Err(transaction_error("original tuple recovery did not verify")); }
        self.secret_store.delete_recovery()?;
        if self.secret_store.load_recovery()?.is_some() {
            return Err(transaction_error("original tuple restored but recovery cleanup is pending"));
        }
        Ok(true)
    }
    fn write_pair(&self, pair: &StoredMeshPair) -> Result<(), MeshStoreError> {
        let current = self.pair()?;
        if current.secret != pair.secret {
        match &pair.secret {
            Some(secret) => self.secret_store.save_secret(secret)?,
            None => self.secret_store.delete_secret()?,
        }
        }
        mesh_boundary("canonical-secret")?;
        if current.metadata != pair.metadata {
        match &pair.metadata {
            Some(value) => write_private_file(&self.config_path, value.as_bytes())?,
            None => remove_file_if_exists(&self.config_path)?,
        }
        }
        mesh_boundary("canonical-metadata")?;
        Ok(())
    }
    fn transact(&self, after: StoredMeshPair) -> Result<(), MeshStoreError> {
        self.refuse_pending()?;
        let before = self.pair()?;
        before.config()?;
        after.config()?;
        if before == after { return Ok(()); }
        let record = MeshRecoveryRecord { profile: self.profile_binding()?, before, after };
        self.secret_store.save_recovery(&record)?;
        mesh_boundary("recovery-staged")?;
        if self.secret_store.load_recovery()?.as_ref() != Some(&record) {
            return Err(transaction_error("protected recovery did not verify; nothing replayed"));
        }
        let result = (|| {
            self.write_pair(&record.after)?;
            if self.pair()? != record.after { return Err(transaction_error("new tuple did not verify")); }
            mesh_boundary("before-commit")?;
            self.secret_store.delete_recovery()?; // Commit point, after both values are durable.
            mesh_boundary("after-commit")?;
            if self.secret_store.load_recovery()?.is_some() {
                return Err(transaction_error("commit cleanup did not verify"));
            }
            Ok(())
        })();
        if result.is_err() {
            match self.secret_store.load_recovery() {
                Ok(Some(_)) => {
                    if self.recover_locked().is_err() {
                        return Err(MeshStoreError::TransactionOutcomeUnknown);
                    }
                }
                Ok(None) => {
                    // The commit may have completed before its acknowledgement
                    // failed. Return a typed outcome, never ordinary rollback
                    // failure: callers must read the saved pair and reconcile.
                    return Err(match self.pair() {
                        Ok(current) if current == record.after => MeshStoreError::CommittedDurabilityUnconfirmed,
                        _ => MeshStoreError::TransactionOutcomeUnknown,
                    });
                }
                Err(_) => return Err(MeshStoreError::TransactionOutcomeUnknown),
            }
        }
        result
    }
    pub fn load(&self) -> Result<Option<MeshConfig>, MeshStoreError> {
        self.validate_access()?;
        if !self.root_dir.exists() { return Ok(None); }
        let _lock = self.lock()?;
        self.recover_locked()?;
        self.pair()?.config()
    }
    pub fn load_or_generate(&self, display_name: impl Into<String>) -> Result<MeshConfig, MeshStoreError> {
        let _lock = self.lock()?;
        let recovered = self.recover_locked()?;
        if let Some(config) = self.pair()?.config()? { return Ok(config); }
        if recovered {
            return Err(transaction_error("interrupted initialization rolled back; no new identity generated"));
        }
        let config = MeshConfig::generate(display_name);
        self.save_locked(&config)?;
        Ok(config)
    }
    /// Explicit owner-side name change. Existing labels (including the product
    /// name) are never guessed to be defaults. Only public metadata is replaced;
    /// membership, node identity and the credential file remain untouched.
    pub fn rename_display_name(&self, expected: &str, name: &str) -> Result<MeshConfig, MeshStoreError> {
        let name = crate::device_name::valid_name(name).ok_or_else(||
            MeshStoreError::InvalidMetadata("invalid device display name".into()))?;
        let _lock = self.lock()?;
        self.refuse_pending()?;
        let mut config = self.pair()?.config()?.ok_or_else(||
            MeshStoreError::InvalidMetadata("device configuration is missing".into()))?;
        if config.display_name != expected {
            return Err(MeshStoreError::InvalidMetadata("device name changed; read current state first".into()));
        }
        if config.display_name != name {
            config.display_name = name;
            write_private_file(&self.config_path, encode_mesh_config_metadata(&config).as_bytes())?;
        }
        Ok(config)
    }

    /// Validate an invitation without writing it. Membership changes never replace
    /// this installation's identity. The boolean reports whether a save/reload is needed.
    pub fn prepare_join(
        &self,
        invite_code: &str,
        display_name: impl Into<String>,
    ) -> Result<(MeshConfig, bool), MeshStoreError> {
        let mut joined = MeshConfig::from_invite_code(invite_code, display_name)
            .map_err(MeshStoreError::InvalidMeshConfig)?;
        if let Some(current) = self.load()? {
            joined.node_id.clone_from(&current.node_id);
            joined.display_name.clone_from(&current.display_name);
            let changed = joined != current;
            Ok((joined, changed))
        } else {
            Ok((joined, true))
        }
    }

    /// Join and preserve installation identity under the same profile lock.
    pub fn join(&self, invite_code: &str, display_name: impl Into<String>) -> Result<(MeshConfig, bool), MeshStoreError> {
        let mut joined = MeshConfig::from_invite_code(invite_code, display_name)
            .map_err(MeshStoreError::InvalidMeshConfig)?;
        let _lock = self.lock()?;
        self.refuse_pending()?;
        if let Some(current) = self.pair()?.config()? {
            joined.node_id.clone_from(&current.node_id);
            joined.display_name.clone_from(&current.display_name);
            if joined == current { return Ok((current, false)); }
        }
        self.save_locked(&joined)?;
        Ok((joined, true))
    }
    fn save_locked(&self, config: &MeshConfig) -> Result<(), MeshStoreError> {
        config.validate().map_err(MeshStoreError::InvalidMeshConfig)?;
        self.transact(StoredMeshPair { metadata: Some(encode_mesh_config_metadata(config)),
            secret: Some(config.network_secret.clone()) })
    }
    pub fn save(&self, config: &MeshConfig) -> Result<(), MeshStoreError> {
        config.validate().map_err(MeshStoreError::InvalidMeshConfig)?;
        let _lock = self.lock()?;
        self.save_locked(config)
    }
    pub fn delete(&self) -> Result<(), MeshStoreError> {
        let _lock = self.lock()?;
        self.transact(StoredMeshPair { metadata: None, secret: None })
    }

}

#[derive(Debug)]
pub enum MeshStoreError {
    Io { path: PathBuf, source: io::Error },
    UnsafeProfile { path: PathBuf, message: String },
    MissingNetworkSecret,
    CommittedDurabilityUnconfirmed,
    TransactionOutcomeUnknown,
    InvalidMetadata(String),
    InvalidMeshConfig(MeshConfigError),
}
impl MeshStoreError {
    fn io(path: PathBuf, source: io::Error) -> Self {
        Self::Io { path, source }
    }
}
impl fmt::Display for MeshStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnsafeProfile { path, message } => write!(f, "{}: {message}", path.display()),
            Self::CommittedDurabilityUnconfirmed => f.write_str("device-group change is saved but storage durability confirmation failed; read current state before continuing"),
            Self::TransactionOutcomeUnknown => f.write_str("device-group change outcome is unknown; recovery retained, do not repeat the change"),
            Self::MissingNetworkSecret => {
                f.write_str("stored device-group config is missing its secret")
            }
            Self::InvalidMetadata(message) => {
                write!(f, "stored device-group config is invalid: {message}")
            }
            Self::InvalidMeshConfig(err) => write!(f, "{err}"),
        }
    }
}
impl Error for MeshStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidMeshConfig(err) => Some(err),
            Self::MissingNetworkSecret | Self::CommittedDurabilityUnconfirmed | Self::TransactionOutcomeUnknown | Self::InvalidMetadata(_) | Self::UnsafeProfile { .. } => {
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshConfigError {
    EmptyNetworkName,
    EmptyNetworkSecret,
    EmptyNodeId,
    EmptyDisplayName,
    InvalidInviteCode,
}
impl fmt::Display for MeshConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNetworkName => f.write_str("device-group network name is empty"),
            Self::EmptyNetworkSecret => f.write_str("device-group secret is empty"),
            Self::EmptyNodeId => f.write_str("device node id is empty"),
            Self::EmptyDisplayName => f.write_str("device display name is empty"),
            Self::InvalidInviteCode => f.write_str("device-group invite code is invalid"),
        }
    }
}
impl Error for MeshConfigError {}

const MESH_CONFIG_METADATA_VERSION: &str = "weavelo-device-group-v2";
const LEGACY_METADATA_VERSION: &str = "weavelo-mesh-v1";
const MESH_CONFIG_PLACEHOLDER_SECRET: &str = "metadata-secret-loaded-separately";

fn encode_mesh_config_metadata(config: &MeshConfig) -> String {
    format!(
        "version={MESH_CONFIG_METADATA_VERSION}\nnetwork_name_hex={}\nnode_id_hex={}\ndisplay_name_hex={}\n",
        hex_bytes(config.network_name.as_bytes()),
        hex_bytes(config.node_id.as_bytes()),
        hex_bytes(config.display_name.as_bytes()),
    )
}

fn decode_mesh_config_metadata(value: &str) -> Result<MeshConfig, MeshStoreError> {
    let mut version = None;
    let mut network_name = None;
    let mut node_id = None;
    let mut display_name = None;
    for (line_idx, raw_line) in value.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            return Err(MeshStoreError::InvalidMetadata(format!(
                "line {} is missing '='",
                line_idx + 1
            )));
        };
        match key {
            "version" => version = Some(raw_value.to_string()),
            "network_name_hex" => network_name = Some(decode_hex_string(raw_value)?),
            "node_id_hex" => node_id = Some(decode_hex_string(raw_value)?),
            "display_name_hex" => display_name = Some(decode_hex_string(raw_value)?),
            _ => {} // old metadata fields are ignored and removed on the next save
        }
    }
    if !matches!(
        version.as_deref(),
        Some(MESH_CONFIG_METADATA_VERSION | LEGACY_METADATA_VERSION)
    ) {
        return Err(MeshStoreError::InvalidMetadata(
            "unsupported or missing version".to_string(),
        ));
    }
    Ok(MeshConfig {
        network_name: required_metadata_string("network_name_hex", network_name)?,
        network_secret: MeshSecret::new(MESH_CONFIG_PLACEHOLDER_SECRET)
            .map_err(MeshStoreError::InvalidMeshConfig)?,
        node_id: required_metadata_string("node_id_hex", node_id)?,
        display_name: required_metadata_string("display_name_hex", display_name)?,
    })
}

fn required_metadata_string(
    key: &'static str,
    value: Option<String>,
) -> Result<String, MeshStoreError> {
    value.ok_or_else(|| MeshStoreError::InvalidMetadata(format!("missing {key}")))
}
fn decode_hex_string(value: &str) -> Result<String, MeshStoreError> {
    String::from_utf8(decode_hex_bytes(value)?)
        .map_err(|_| MeshStoreError::InvalidMetadata("hex value is not valid utf-8".to_string()))
}
fn decode_hex_bytes(value: &str) -> Result<Vec<u8>, MeshStoreError> {
    if !value.len().is_multiple_of(2) {
        return Err(MeshStoreError::InvalidMetadata(
            "hex value has odd length".to_string(),
        ));
    }
    let raw = value.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    for idx in (0..raw.len()).step_by(2) {
        let high = decode_hex_nibble(raw[idx])?;
        let low = decode_hex_nibble(raw[idx + 1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}
fn decode_hex_nibble(value: u8) -> Result<u8, MeshStoreError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(MeshStoreError::InvalidMetadata(
            "hex value contains non-hex digit".to_string(),
        )),
    }
}

fn unsafe_profile(path: &Path, message: impl Into<String>) -> MeshStoreError {
    MeshStoreError::UnsafeProfile {
        path: path.to_path_buf(),
        message: message.into(),
    }
}
#[cfg(all(test, unix))]
thread_local! {
    static PROFILE_TEST_UID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}
#[cfg(unix)]
fn profile_uid() -> u32 {
    #[cfg(test)]
    if let Some(uid) = PROFILE_TEST_UID.with(|value| value.get()) {
        return uid;
    }
    // Read the actual effective identity, never an overridable UID/HOME variable.
    unsafe { libc::geteuid() }
}
pub(crate) fn check_profile_path(path: &Path) -> Result<(), MeshStoreError> {
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(unsafe_profile(
            path,
            "profile paths may not contain parent traversal; nothing changed",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?
            .join(path)
    };
    for ancestor in absolute.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                // macOS /var and /tmp (and Android's system storage prefixes)
                // can be root-controlled OS links. A profile leaf or a user's
                // link inside its ancestry is never followed.
                #[cfg(unix)]
                if ancestor != absolute && metadata.uid() == 0 {
                    continue;
                }
                return Err(unsafe_profile(
                    ancestor,
                    "profile path contains a symlink; nothing changed",
                ));
            }
            Ok(metadata) =>
            {
                #[cfg(unix)]
                if profile_uid() == 0 && metadata.uid() != 0 {
                    return Err(unsafe_profile(
                        ancestor,
                        "root must not initialize or rewrite a profile beneath another user's directory; explicit administrator recovery is required; nothing changed",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(MeshStoreError::io(ancestor.to_path_buf(), source)),
        }
    }
    Ok(())
}
fn check_owner(path: &Path, metadata: &fs::Metadata) -> Result<(), MeshStoreError> {
    #[cfg(unix)]
    if metadata.uid() != profile_uid() {
        return Err(unsafe_profile(
            path,
            format!(
                "profile owner UID {} differs from process UID {}; run as its owner and arrange explicit administrator recovery if ownership is wrong; nothing changed",
                metadata.uid(),
                profile_uid(),
            ),
        ));
    }
    Ok(())
}
fn validate_private_dir(path: &Path) -> Result<(), MeshStoreError> {
    check_profile_path(path)?;
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(unsafe_profile(
                        ancestor,
                        "profile parent is not an ordinary directory; nothing changed",
                    ));
                }
                check_owner(ancestor, &metadata)?;
                #[cfg(unix)]
                if ancestor == path && metadata.mode() & 0o077 != 0 {
                    return Err(unsafe_profile(
                        path,
                        "profile directory is not private; permissions were not changed",
                    ));
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(MeshStoreError::io(ancestor.to_path_buf(), source)),
        }
    }
    Err(unsafe_profile(
        path,
        "profile has no owned existing parent; nothing changed",
    ))
}
fn validate_private_parent(path: &Path) -> Result<(), MeshStoreError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        validate_private_dir(parent)
    } else {
        validate_private_dir(
            &env::current_dir().map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?,
        )
    }
}
fn private_file_metadata(path: &Path) -> Result<Option<fs::Metadata>, MeshStoreError> {
    check_profile_path(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(MeshStoreError::io(path.to_path_buf(), source)),
    };
    check_private_file(path, &metadata)?;
    Ok(Some(metadata))
}
fn check_private_file(path: &Path, metadata: &fs::Metadata) -> Result<(), MeshStoreError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(unsafe_profile(
            path,
            "profile entry is not an ordinary file; nothing changed",
        ));
    }
    check_owner(path, metadata)?;
    #[cfg(unix)]
    if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
        return Err(unsafe_profile(
            path,
            "profile file must be private and have one link; permissions were not changed",
        ));
    }
    Ok(())
}
fn same_profile_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    if (a.dev(), a.ino(), a.uid(), a.mode()) != (b.dev(), b.ino(), b.uid(), b.mode()) {
        return false;
    }
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}
fn read_private_file(path: &Path) -> Result<Option<String>, MeshStoreError> {
    validate_private_parent(path)?;
    let Some(before) = private_file_metadata(path)? else {
        return Ok(None);
    };
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
    let opened = file
        .metadata()
        .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
    check_private_file(path, &opened)?;
    if !same_profile_file(&before, &opened)
        || private_file_metadata(path)?
            .as_ref()
            .is_none_or(|now| !same_profile_file(now, &opened))
    {
        return Err(unsafe_profile(
            path,
            "profile changed while opening; nothing changed",
        ));
    }
    if opened.len() > PRIVATE_ENTRY_LIMIT { return Err(transaction_error("profile entry exceeds size limit")); }
    let mut text = String::new();
    file.take(PRIVATE_ENTRY_LIMIT + 1).read_to_string(&mut text)
        .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
    if text.len() as u64 > PRIVATE_ENTRY_LIMIT { return Err(transaction_error("profile entry exceeds size limit")); }
    Ok(Some(text))
}
pub(crate) fn create_private_dir(path: &Path) -> Result<(), MeshStoreError> {
    validate_private_dir(path)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
    validate_private_dir(path)
}
fn publish_new_private_file(temporary: &Path, path: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(temporary.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid profile path"))?;
        let target = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid profile path"))?;
        // Android SELinux forbids app-data hard links. Never replace an identity
        // that another initializer published after the access check.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(temporary.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let target = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        if unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" { fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32; }
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // WRITE_THROUGH, deliberately no REPLACE_EXISTING.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 8) } == 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }
    #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "macos", windows)))]
    { Err(io::Error::new(io::ErrorKind::Unsupported, "atomic private publication is unsupported")) }
}
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), MeshStoreError> {
    validate_private_parent(path)?;
    let before = private_file_metadata(path)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_private_dir(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("device-group-store");
    let temporary = path.with_file_name(format!(".{file_name}.tmp-{}", hex_random::<4>()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(before.as_ref().map(|m| m.mode() & 0o777).unwrap_or(0o600));
        let mut file = options
            .open(&temporary)
            .map_err(|source| MeshStoreError::io(temporary.clone(), source))?;
        mesh_boundary("file-created")?;
        file.write_all(bytes).map_err(|source| MeshStoreError::io(temporary.clone(), source))?;
        mesh_boundary("file-written")?;
        file.sync_all().map_err(|source| MeshStoreError::io(temporary.clone(), source))?;
        mesh_boundary("file-synced")?;
        if let Some(previous) = &before {
            file.set_permissions(previous.permissions())
                .and_then(|_| file.sync_all())
                .map_err(|source| MeshStoreError::io(temporary.clone(), source))?;
            mesh_boundary("permissions-synced")?;
        }
        validate_private_parent(path)?;
        let current = private_file_metadata(path)?;
        if !match (&before, &current) {
            (Some(a), Some(b)) => same_profile_file(a, b),
            (None, None) => true,
            _ => false,
        } {
            return Err(unsafe_profile(
                path,
                "profile changed before saving; existing identity retained",
            ));
        }
        if before.is_some() {
            fs::rename(&temporary, path)
                .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
        } else {
            // Atomic create-only publication: a concurrent first start cannot
            // replace an identity that appeared after the access check.
            publish_new_private_file(&temporary, path)
                .map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
        }
        mesh_boundary("file-published")?;
        sync_private_parent(path)?;
        mesh_boundary("directory-synced")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn remove_file_if_exists(path: &Path) -> Result<(), MeshStoreError> {
    validate_private_parent(path)?;
    if private_file_metadata(path)?.is_none() {
        return Ok(());
    }
    mesh_boundary("before-remove")?;
    fs::remove_file(path).map_err(|source| MeshStoreError::io(path.to_path_buf(), source))?;
    mesh_boundary("file-removed")?;
    sync_private_parent(path)?;
    mesh_boundary("remove-directory-synced")?;
    Ok(())
}
fn sync_private_parent(path: &Path) -> Result<(), MeshStoreError> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent).and_then(|file| file.sync_all())
            .map_err(|e| MeshStoreError::io(parent.to_path_buf(), e))?;
    }
    Ok(())
}
// Isolated tests interrupt immediately after actual filesystem/credential steps;
// no environment hook or production failpoint exists.
#[cfg(test)]
thread_local! {
    static MESH_BOUNDARIES: std::cell::RefCell<(Option<(usize, bool)>, Vec<&'static str>)> = const { std::cell::RefCell::new((None, Vec::new())) };
}
fn mesh_boundary(_name: &'static str) -> Result<(), MeshStoreError> {
    #[cfg(test)]
    return MESH_BOUNDARIES.with(|state| {
        let mut state = state.borrow_mut();
        state.1.push(_name);
        if let Some((index, crash)) = state.0 {
            if index == state.1.len() {
                state.0 = None;
                drop(state);
                if crash { std::panic::panic_any("synthetic mesh interruption"); }
                return Err(transaction_error("synthetic mesh write failure"));
            }
        }
        Ok(())
    });
    #[cfg(not(test))]
    Ok(())
}
fn non_empty_env_path(value: std::ffi::OsString) -> Option<std::ffi::OsString> {
    (!value.to_string_lossy().trim().is_empty()).then_some(value)
}
fn sanitized_display_name(display_name: impl Into<String>) -> String {
    let value = display_name.into();
    let value = value.trim();
    if value.is_empty() {
        crate::device_name::host_name()
    } else {
        value.to_string()
    }
}
fn hex_random<const N: usize>() -> String {
    hex_bytes(&random::<[u8; N]>())
}
fn hex_bytes(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_ID: AtomicU64 = AtomicU64::new(1);
    struct TempTree {
        root: PathBuf,
    }
    impl TempTree {
        fn new(name: &str) -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let root =
                env::temp_dir().join(format!("weavelo-{name}-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            Self { root }
        }
    }
    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn profile_first_publication_never_overwrites_an_existing_identity() {
        let temp = TempTree::new("profile-publish-existing");
        let target = temp.root.join("identity");
        let staged = temp.root.join("staged");
        write_private_file(&target, b"existing identity").unwrap();
        write_private_file(&staged, b"candidate identity").unwrap();
        let before = fs::read(&target).unwrap();
        let error = publish_new_private_file(&staged, &target).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), before);
        assert_eq!(fs::read(&staged).unwrap(), b"candidate identity");
        assert_eq!(fs::read_dir(&temp.root).unwrap().count(), 2);
    }

    #[test]
    fn profile_concurrent_first_publication_has_exactly_one_complete_winner() {
        use std::sync::{Arc, Barrier};
        let temp = TempTree::new("profile-publish-race");
        let target = temp.root.join("identity");
        let barrier = Arc::new(Barrier::new(2));
        let workers: Vec<_> = [b"first identity".as_slice(), b"second identity".as_slice()]
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| {
                let staged = temp.root.join(format!("staged-{index}"));
                write_private_file(&staged, bytes).unwrap();
                let target = target.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = publish_new_private_file(&staged, &target);
                    if result.is_err() {
                        fs::remove_file(staged).unwrap();
                    }
                    (result, bytes)
                })
            })
            .collect();
        let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|(result, _)| result.is_ok()).count(), 1);
        let winner = results.iter().find(|(result, _)| result.is_ok()).unwrap().1;
        assert_eq!(fs::read(&target).unwrap(), winner);
        assert_eq!(fs::read_dir(&temp.root).unwrap().count(), 1);
        check_private_file(&target, &fs::metadata(&target).unwrap()).unwrap();
        for (result, _) in results {
            if let Err(error) = result {
                assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            }
        }
    }

    #[test]
    fn explicit_name_change_preserves_membership_and_custom_names_on_reload() {
        let temp = TempTree::new("device-name");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = store.load_or_generate("Weavelo").unwrap();
        let after = store.rename_display_name("Weavelo", "Studio").unwrap();
        let mut expected = before.clone();
        expected.display_name = "Studio".into();
        assert_eq!(after, expected);
        assert_eq!(store.load_or_generate("Another host").unwrap(), expected);
        assert!(store.rename_display_name("Weavelo", "wrong").is_err());
        assert!(store.rename_display_name("Studio", "line\nfeed").is_err());
        assert_eq!(store.load().unwrap().unwrap(), expected);
        let custom = store.rename_display_name("Studio", "Weavelo").unwrap();
        assert_eq!(store.load_or_generate("Host fallback").unwrap(), custom);
    }

    #[test]
    fn missing_default_name_uses_host_label_without_reusing_device_identity() {
        let first = MeshConfig::generate("");
        let second = MeshConfig::generate("   ");
        assert_eq!(first.display_name, crate::device_name::host_name());
        assert_eq!(second.display_name, first.display_name);
        assert_ne!(first.node_id, second.node_id);
        first.validate().unwrap();
        second.validate().unwrap();
    }

    #[test]
    fn generated_config_is_valid_and_redacts_secret() {
        let config = MeshConfig::generate("Alice Mac");
        config.validate().unwrap();
        assert!(config.network_name.starts_with("weavelo-"));
        assert_eq!(config.display_name, "Alice Mac");
        assert!(
            !format!("{:?}", config.network_secret).contains(config.network_secret.expose_secret())
        );
    }

    #[test]
    fn invite_roundtrips_group_credentials_without_node_identity() {
        let config = MeshConfig::generate("Host");
        let code = config.invite_code();
        assert!(code.starts_with(WEAVELO_DEVICE_GROUP_COPY_CODE_PREFIX));
        assert!(!code.contains(config.network_secret.expose_secret()));
        let joined = MeshConfig::from_invite_code(&code, "Viewer").unwrap();
        assert_eq!(joined.network_name, config.network_name);
        assert_eq!(
            joined.network_secret.expose_secret(),
            config.network_secret.expose_secret()
        );
        assert_ne!(joined.node_id, config.node_id);
        assert_eq!(joined.display_name, "Viewer");
    }

    #[test]
    fn invite_tolerates_case_whitespace_and_grouping() {
        let config = MeshConfig::generate("Host");
        let noisy = config
            .invite_code()
            .chars()
            .enumerate()
            .map(|(idx, ch)| {
                if idx % 11 == 0 {
                    format!(" \n{}", ch.to_ascii_lowercase())
                } else {
                    ch.to_ascii_lowercase().to_string()
                }
            })
            .collect::<String>();
        let joined = MeshConfig::from_invite_code(&noisy, "Viewer").unwrap();
        assert_eq!(joined.network_name, config.network_name);
    }

    #[test]
    fn invite_rejects_checksum_changes() {
        let config = MeshConfig::generate("Host");
        let mut code = compact_copy_code(&config.invite_code());
        // The last Base32 symbol may contain padding bits, so changing it
        // does not necessarily change the checksum bytes. The penultimate
        // symbol always contains five payload bits.
        let tail = code.pop().unwrap();
        let checksum_symbol = code.pop().unwrap();
        code.push(if checksum_symbol == 'A' { 'B' } else { 'A' });
        code.push(tail);
        assert_eq!(
            MeshInvite::decode(&code).unwrap_err(),
            MeshConfigError::InvalidInviteCode
        );
    }

    #[test]
    fn joining_network_preserves_installation_identity() {
        let temp = TempTree::new("join-identity");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let local = MeshConfig::generate("My phone");
        let remote = MeshConfig::generate("Desktop");
        store.save(&local).unwrap();
        let (joined, changed) = store
            .prepare_join(&remote.invite_code(), "Default name")
            .unwrap();
        assert!(changed);
        assert_eq!(joined.node_id, local.node_id);
        assert_ne!(joined.node_id, remote.node_id);
        assert_eq!(joined.display_name, local.display_name);
        assert_eq!(joined.network_name, remote.network_name);
        assert_eq!(joined.network_secret, remote.network_secret);
        assert_eq!(
            store.load().unwrap().unwrap(),
            local,
            "prepare must not write"
        );
        store.save(&joined).unwrap();
        let (repeated, changed) = store
            .prepare_join(&remote.invite_code(), "Ignored")
            .unwrap();
        assert!(!changed, "repeat scan must not restart the network");
        assert_eq!(repeated, joined);
        assert_eq!(store.load().unwrap().unwrap(), joined);
    }

    #[test]
    fn invalid_join_keeps_existing_membership() {
        let temp = TempTree::new("join-invalid");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let local = MeshConfig::generate("Local");
        store.save(&local).unwrap();
        assert!(store.prepare_join("invalid invite", "Ignored").is_err());
        assert_eq!(store.load().unwrap().unwrap(), local);
    }

    #[test]
    fn joining_rotated_credentials_is_not_a_noop() {
        let temp = TempTree::new("join-rotation");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let local = MeshConfig::generate("Local");
        let mut rotated = MeshConfig::generate("Other device");
        rotated.network_name.clone_from(&local.network_name);
        store.save(&local).unwrap();
        let (joined, changed) = store
            .prepare_join(&rotated.invite_code(), "Ignored")
            .unwrap();
        assert!(changed);
        assert_eq!(joined.node_id, local.node_id);
        assert_eq!(joined.network_secret, rotated.network_secret);
    }

    #[test]
    fn first_join_is_prepared_without_creating_files() {
        let temp = TempTree::new("join-first");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let remote = MeshConfig::generate("Desktop");
        let (joined, changed) = store.prepare_join(&remote.invite_code(), "Phone").unwrap();
        assert!(changed);
        assert_eq!(joined.display_name, "Phone");
        assert_ne!(joined.node_id, remote.node_id);
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn store_roundtrips_config_and_secret() {
        let temp = TempTree::new("group-store");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let config = MeshConfig::generate("Desk");
        let secret = config.network_secret.expose_secret().to_string();
        store.save(&config).unwrap();
        let metadata = fs::read_to_string(store.config_path()).unwrap();
        assert!(!metadata.contains(&secret));
        assert_eq!(store.load().unwrap().unwrap(), config);
        store.delete().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn store_preserves_legacy_metadata_until_explicit_save() {
        let temp = TempTree::new("group-migrate");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let config = MeshConfig::generate("Desk");
        let metadata = format!(
            "version={LEGACY_METADATA_VERSION}\nnetwork_name_hex={}\nnode_id_hex={}\ndisplay_name_hex={}\nauto_start=1\nintegration_mode=old\ninitial_peer_hex=00\n",
            hex_bytes(config.network_name.as_bytes()),
            hex_bytes(config.node_id.as_bytes()),
            hex_bytes(config.display_name.as_bytes())
        );
        write_private_file(store.config_path(), metadata.as_bytes()).unwrap();
        store
            .secret_store()
            .save_secret(&config.network_secret)
            .unwrap();
        let loaded = store.load_or_generate("Ignored").unwrap();
        assert_eq!(loaded.network_name, config.network_name);
        assert_eq!(fs::read_to_string(store.config_path()).unwrap(), metadata);
        store.save(&loaded).unwrap(); // Explicit save, never ordinary startup.
        let normalized = fs::read_to_string(store.config_path()).unwrap();
        assert!(normalized.contains(MESH_CONFIG_METADATA_VERSION));
        assert!(!normalized.contains("initial_peer"));
        assert!(!normalized.contains("integration_mode"));
    }

    #[cfg(unix)]
    #[test]
    fn store_uses_private_permissions() {
        let temp = TempTree::new("group-permissions");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        store.save(&MeshConfig::generate("Desk")).unwrap();
        assert_eq!(
            fs::metadata(store.root_dir()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.config_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.secret_store().path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[cfg(unix)]
    fn profile_snapshot(path: &Path) -> (u32, u32, u64, std::time::SystemTime, Vec<u8>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        (
            metadata.uid(),
            metadata.mode() & 0o777,
            metadata.ino(),
            metadata.modified().unwrap(),
            fs::read(path).unwrap(),
        )
    }
    #[cfg(unix)]
    fn with_profile_uid<T>(uid: u32, run: impl FnOnce() -> T) -> T {
        struct Restore(Option<u32>);
        impl Drop for Restore {
            fn drop(&mut self) {
                PROFILE_TEST_UID.with(|v| v.set(self.0));
            }
        }
        let previous = PROFILE_TEST_UID.with(|v| v.replace(Some(uid)));
        let _restore = Restore(previous);
        run()
    }
    #[cfg(unix)]
    #[test]
    fn profile_first_user_start_creates_one_private_identity_and_reuses_it() {
        let temp = TempTree::new("first-user-profile");
        let store = AppPrivateMeshConfigStore::new(temp.root.join("new-profile"));
        let first = store.load_or_generate("First user").unwrap();
        let config_before = profile_snapshot(store.config_path());
        let secret_before = profile_snapshot(store.secret_store().path());
        let second = store
            .load_or_generate("Must not replace user identity")
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(config_before, profile_snapshot(store.config_path()));
        assert_eq!(secret_before, profile_snapshot(store.secret_store().path()));
        assert_eq!(config_before.0, profile_uid());
        assert_eq!(config_before.1, 0o600);
        assert_eq!(secret_before.1, 0o600);
    }
    #[cfg(unix)]
    #[test]
    fn profile_root_install_context_cannot_initialize_user_directory_then_user_can_start() {
        let temp = TempTree::new("root-install-context");
        let store = AppPrivateMeshConfigStore::new(temp.root.join("profile"));
        // Simulate only the caller identity; never setuid/chown or execute sudo.
        let installer_uid = if profile_uid() == 0 { 1 } else { 0 };
        let error =
            with_profile_uid(installer_uid, || store.load_or_generate("Installer")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("explicit administrator recovery")
        );
        assert!(!store.root_dir().exists());
        let user = store.load_or_generate("User").unwrap();
        assert_eq!(user.display_name, "User");
        assert_eq!(
            fs::metadata(store.config_path()).unwrap().uid(),
            profile_uid()
        );
    }
    #[cfg(unix)]
    #[test]
    fn profile_foreign_caller_cannot_rewrite_read_or_delete_existing_identity() {
        let temp = TempTree::new("foreign-profile-context");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let config = store.load_or_generate("User").unwrap();
        let config_before = profile_snapshot(store.config_path());
        let secret_before = profile_snapshot(store.secret_store().path());
        let directory_mode = fs::metadata(&temp.root).unwrap().mode();
        with_profile_uid(profile_uid().wrapping_add(1), || {
            assert!(store.load().is_err());
            assert!(store.load_or_generate("Other").is_err());
            assert!(store.save(&MeshConfig::generate("Other")).is_err());
            assert!(store.delete().is_err());
        });
        assert_eq!(config_before, profile_snapshot(store.config_path()));
        assert_eq!(secret_before, profile_snapshot(store.secret_store().path()));
        assert_eq!(directory_mode, fs::metadata(&temp.root).unwrap().mode());
        assert_eq!(store.load().unwrap().unwrap(), config);
    }
    #[cfg(unix)]
    #[test]
    fn profile_existing_read_only_files_and_directory_are_not_widened_by_startup() {
        let temp = TempTree::new("readonly-profile");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let first = store.load_or_generate("User").unwrap();
        for path in [store.config_path(), store.secret_store().path()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
        }
        fs::set_permissions(&temp.root, fs::Permissions::from_mode(0o500)).unwrap();
        let before = (
            profile_snapshot(store.config_path()),
            profile_snapshot(store.secret_store().path()),
        );
        assert_eq!(store.load_or_generate("Ignored").unwrap(), first);
        assert_eq!(
            before,
            (
                profile_snapshot(store.config_path()),
                profile_snapshot(store.secret_store().path())
            )
        );
        assert_eq!(fs::metadata(&temp.root).unwrap().mode() & 0o777, 0o500);
        fs::set_permissions(&temp.root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn profile_legacy_startup_preserves_both_files_inode_content_and_permissions() {
        let temp = TempTree::new("legacy-secret-preservation");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let config = store.load_or_generate("User").unwrap();
        let text = encode_mesh_config_metadata(&config)
            .replace(MESH_CONFIG_METADATA_VERSION, LEGACY_METADATA_VERSION);
        write_private_file(store.config_path(), text.as_bytes()).unwrap();
        fs::set_permissions(store.config_path(), fs::Permissions::from_mode(0o400)).unwrap();
        let config_before = profile_snapshot(store.config_path());
        let secret_before = profile_snapshot(store.secret_store().path());
        assert_eq!(store.load_or_generate("Ignored").unwrap(), config);
        assert_eq!(config_before, profile_snapshot(store.config_path()));
        assert_eq!(secret_before, profile_snapshot(store.secret_store().path()));
        assert_eq!(
            fs::metadata(store.config_path()).unwrap().mode() & 0o777,
            0o400
        );
    }
    #[cfg(unix)]
    #[test]
    fn profile_orphan_secret_is_retained_instead_of_rotating_membership() {
        let temp = TempTree::new("orphan-secret");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        store
            .secret_store()
            .save_secret(&MeshSecret::generate())
            .unwrap();
        let before = profile_snapshot(store.secret_store().path());
        let error = store.load_or_generate("Do not regenerate").unwrap_err();
        assert!(error.to_string().contains("existing secret is retained"));
        assert!(!store.config_path().exists());
        assert_eq!(before, profile_snapshot(store.secret_store().path()));
    }
    #[cfg(unix)]
    #[test]
    fn profile_config_and_secret_symlinks_are_rejected_before_other_file_changes() {
        use std::os::unix::fs::symlink;
        for name in [MESH_CONFIG_FILE_NAME, MESH_SECRET_FILE_NAME] {
            let temp = TempTree::new("profile-file-link");
            let store = AppPrivateMeshConfigStore::new(temp.root.join("profile"));
            let config = store.load_or_generate("User").unwrap();
            let path = store.root_dir().join(name);
            let target = temp.root.join("unrelated-file");
            fs::rename(&path, &target).unwrap();
            symlink(&target, &path).unwrap();
            let other = if name == MESH_CONFIG_FILE_NAME {
                store.secret_store().path()
            } else {
                store.config_path()
            };
            let target_before = profile_snapshot(&target);
            let other_before = profile_snapshot(other);
            assert!(store.load_or_generate("Other").is_err());
            assert!(store.save(&config).is_err());
            assert!(store.delete().is_err());
            assert!(
                fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(target_before, profile_snapshot(&target));
            assert_eq!(other_before, profile_snapshot(other));
        }
    }
    #[cfg(unix)]
    #[test]
    fn profile_root_links_parent_links_and_traversal_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = TempTree::new("profile-directory-link");
        let owned = temp.root.join("owned");
        create_private_dir(&owned).unwrap();
        let link = temp.root.join("linked");
        symlink(&owned, &link).unwrap();
        for path in [
            link.clone(),
            link.join("nested"),
            temp.root
                .join("..")
                .join(temp.root.file_name().unwrap())
                .join("new"),
        ] {
            let store = AppPrivateMeshConfigStore::new(path);
            assert!(store.load_or_generate("Other").is_err());
        }
        assert_eq!(fs::read_dir(&owned).unwrap().count(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn profile_hardlinked_secret_and_public_permissions_are_retained_on_rejection() {
        let temp = TempTree::new("profile-unsafe-files");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        store.load_or_generate("User").unwrap();
        let alias = temp.root.join("alias");
        fs::hard_link(store.secret_store().path(), &alias).unwrap();
        let before = profile_snapshot(store.config_path());
        assert!(store.load_or_generate("Other").is_err());
        assert!(store.delete().is_err());
        assert_eq!(before, profile_snapshot(store.config_path()));
        fs::remove_file(alias).unwrap();
        fs::set_permissions(store.config_path(), fs::Permissions::from_mode(0o644)).unwrap();
        let public_before = profile_snapshot(store.config_path());
        assert!(store.load_or_generate("Other").is_err());
        assert_eq!(public_before, profile_snapshot(store.config_path()));
    }
    fn boundary_run<T>(at: Option<(usize, bool)>, run: impl FnOnce() -> T) -> (T, Vec<&'static str>) {
        MESH_BOUNDARIES.with(|state| *state.borrow_mut() = (at, Vec::new()));
        let result = run();
        let trace = MESH_BOUNDARIES.with(|state| std::mem::take(&mut state.borrow_mut().1));
        MESH_BOUNDARIES.with(|state| state.borrow_mut().0 = None);
        (result, trace)
    }

    #[test]
    fn transaction_every_file_boundary_is_coherent_after_restart_or_reported_error() {
        for operation in ["join", "initialize", "delete"] {
            let run = |store: &AppPrivateMeshConfigStore, target: &MeshConfig| match operation {
                "delete" => store.delete(),
                "join" => store.join(&target.invite_code(), "ignored").map(|_| ()),
                _ => store.save(target),
            };
            let seed = MeshConfig::generate("原名称");
            let target = MeshConfig::generate("合成目标");
            let baseline = TempTree::new("transaction-baseline");
            let store = AppPrivateMeshConfigStore::new(&baseline.root);
            if operation != "initialize" { store.save(&seed).unwrap(); }
            let (result, trace) = boundary_run(None, || run(&store, &target));
            result.unwrap();
            assert!(trace.contains(&"file-written") && trace.contains(&"after-commit"));
            for crash in [false, true] {
                for index in 1..=trace.len() {
                    let temp = TempTree::new("transaction-interrupt");
                    let store = AppPrivateMeshConfigStore::new(&temp.root);
                    if operation != "initialize" { store.save(&seed).unwrap(); }
                    let (result, _) = boundary_run(Some((index, crash)), ||
                        std::panic::catch_unwind(|| run(&store, &target)));
                    assert!(if crash { result.is_err() } else { result.unwrap().is_err() });
                    let restarted = AppPrivateMeshConfigStore::new(&temp.root);
                    let recovered = restarted.load().unwrap();
                    match operation {
                        "initialize" => assert!(recovered.is_none() || recovered.as_ref() == Some(&target)),
                        "delete" => assert!(recovered.is_none() || recovered.as_ref() == Some(&seed)),
                        _ => {
                            let config = recovered.unwrap();
                            assert_eq!(config.node_id, seed.node_id);
                            assert_eq!(config.display_name, seed.display_name);
                            assert!((config.network_name == seed.network_name && config.network_secret == seed.network_secret)
                                || (config.network_name == target.network_name && config.network_secret == target.network_secret));
                        }
                    }
                    assert!(restarted.secret_store.load_recovery().unwrap().is_none());
                    assert_eq!(store.root_dir(), restarted.root_dir());
                }
            }
        }
    }

    fn stage_mixed(store: &AppPrivateMeshConfigStore, before: &MeshConfig, after: &MeshConfig) -> MeshRecoveryRecord {
        let record = MeshRecoveryRecord {
            profile: store.profile_binding().unwrap(),
            before: StoredMeshPair { metadata: Some(encode_mesh_config_metadata(before)), secret: Some(before.network_secret.clone()) },
            after: StoredMeshPair { metadata: Some(encode_mesh_config_metadata(after)), secret: Some(after.network_secret.clone()) },
        };
        store.secret_store.save_recovery(&record).unwrap();
        store.secret_store.save_secret(&after.network_secret).unwrap();
        record
    }

    #[test]
    fn interrupted_rollback_is_restartable_at_every_boundary_and_never_replays_join() {
        let before = MeshConfig::generate("保留名称");
        let after = MeshConfig::generate("Remote");
        let baseline = TempTree::new("recovery-baseline");
        let store = AppPrivateMeshConfigStore::new(&baseline.root);
        store.save(&before).unwrap();
        stage_mixed(&store, &before, &after);
        let (_, trace) = boundary_run(None, || store.load().unwrap());
        for index in 1..=trace.len() {
            let temp = TempTree::new("recovery-interrupted");
            let store = AppPrivateMeshConfigStore::new(&temp.root);
            store.save(&before).unwrap();
            let record = stage_mixed(&store, &before, &after);
            assert!(store.join(&after.invite_code(), "ignored").is_err());
            assert!(store.save(&after).is_err());
            assert!(store.rename_display_name(&before.display_name, "changed").is_err());
            assert!(store.delete().is_err());
            assert_eq!(store.secret_store.load_recovery().unwrap().as_ref(), Some(&record));
            let (result, _) = boundary_run(Some((index, true)), || std::panic::catch_unwind(|| store.load()));
            assert!(result.is_err());
            assert_eq!(store.load_or_generate("ignored").unwrap(), before);
        }
    }

    #[test]
    fn unknown_recovery_is_retained_without_touching_files_or_exposing_payload() {
        let temp = TempTree::new("recovery-unknown");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = MeshConfig::generate("Local");
        let after = MeshConfig::generate("Other");
        store.save(&before).unwrap();
        let record = stage_mixed(&store, &before, &after);
        assert!(!format!("{record:?}").contains(before.network_secret.expose_secret()));
        assert!(!format!("{record:?}").contains(after.network_secret.expose_secret()));
        let unknown = MeshSecret::generate();
        store.secret_store.save_secret(&unknown).unwrap();
        let metadata = fs::read(store.config_path()).unwrap();
        let recovery = fs::read(store.secret_store.recovery_path()).unwrap();
        let error = store.load().unwrap_err();
        assert!(!format!("{error:?} {error}").contains(unknown.expose_secret()));
        assert_eq!(fs::read(store.config_path()).unwrap(), metadata);
        assert_eq!(fs::read(store.secret_store.recovery_path()).unwrap(), recovery);
        assert_eq!(store.secret_store.load_secret().unwrap(), Some(unknown));
        write_private_file(&store.secret_store.recovery_path(), b"{synthetic-private-payload").unwrap();
        assert!(!format!("{:?}", store.load().unwrap_err()).contains("synthetic-private-payload"));
    }

    #[test]
    fn interrupted_initialization_does_not_generate_another_identity() {
        let temp = TempTree::new("recovery-initialize");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let after = MeshConfig::generate("Synthetic");
        let lock = store.lock().unwrap();
        store.secret_store.save_recovery(&MeshRecoveryRecord {
            profile: store.profile_binding().unwrap(),
            before: StoredMeshPair { metadata: None, secret: None },
            after: StoredMeshPair { metadata: Some(encode_mesh_config_metadata(&after)), secret: Some(after.network_secret.clone()) },
        }).unwrap();
        store.secret_store.save_secret(&after.network_secret).unwrap();
        drop(lock);
        assert!(store.load_or_generate("must not generate").is_err());
        assert!(store.load().unwrap().is_none());
        assert!(store.secret_store.load_secret().unwrap().is_none());
    }

    #[test]
    fn profile_lock_refuses_other_readers_writers_and_join_without_waiting() {
        let temp = TempTree::new("transaction-busy");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = store.load_or_generate("Original").unwrap();
        let lock = store.lock().unwrap();
        let other = store.clone();
        let target = MeshConfig::generate("Remote");
        std::thread::spawn(move || {
            assert!(other.load().is_err());
            assert!(other.save(&target).is_err());
            assert!(other.join(&target.invite_code(), "ignored").is_err());
            assert!(other.delete().is_err());
        }).join().unwrap();
        drop(lock);
        assert_eq!(store.load().unwrap().unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn profile_lock_scope_releases_even_with_an_inherited_description() {
        let temp = TempTree::new("transaction-inherited-lock");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = store.load_or_generate("Original").unwrap();
        let lock = store.lock().unwrap();
        // dup models the same open-file-description reference retained across
        // fork, deterministically and without spawning another test process.
        let inherited = lock.0.try_clone().unwrap();
        assert!(store.load().is_err());
        drop(lock);
        assert_eq!(store.load().unwrap().unwrap(), before);
        let next_owner = store.lock().unwrap();
        drop(inherited);
        // Closing an old inherited description must not unlock a later owner's
        // independent open description.
        assert!(store.load().is_err());
        drop(next_owner);
        assert_eq!(store.load().unwrap().unwrap(), before);
    }

    #[derive(Clone, Default)]
    struct MockKeyring(std::sync::Arc<std::sync::Mutex<(Option<MeshSecret>, Option<MeshRecoveryRecord>)>>);
    impl MeshSecretStore for MockKeyring {
        fn storage_kind(&self) -> MeshSecretStorageKind { MeshSecretStorageKind::PlatformSecureStore }
        fn load_secret(&self) -> Result<Option<MeshSecret>, MeshStoreError> { Ok(self.0.lock().unwrap().0.clone()) }
        fn save_secret(&self, secret: &MeshSecret) -> Result<(), MeshStoreError> {
            self.0.lock().unwrap().0 = Some(secret.clone()); mesh_boundary("keyring-secret-saved")
        }
        fn delete_secret(&self) -> Result<(), MeshStoreError> {
            self.0.lock().unwrap().0 = None; mesh_boundary("keyring-secret-deleted")
        }
        fn load_recovery(&self) -> Result<Option<MeshRecoveryRecord>, MeshStoreError> { Ok(self.0.lock().unwrap().1.clone()) }
        fn save_recovery(&self, record: &MeshRecoveryRecord) -> Result<(), MeshStoreError> {
            self.0.lock().unwrap().1 = Some(record.clone()); mesh_boundary("keyring-recovery-saved")
        }
        fn delete_recovery(&self) -> Result<(), MeshStoreError> {
            self.0.lock().unwrap().1 = None; mesh_boundary("keyring-recovery-deleted")
        }
    }

    #[test]
    fn keyring_recovery_never_downgrades_secrets_into_metadata_or_journal_files() {
        let before = MeshConfig::generate("原名称");
        let after = MeshConfig::generate("Remote");
        let baseline = TempTree::new("keyring-baseline");
        let store = AppPrivateMeshConfigStore::with_secret_store(&baseline.root, MockKeyring::default());
        store.save(&before).unwrap();
        let (result, trace) = boundary_run(None, || store.join(&after.invite_code(), "ignored"));
        result.unwrap();
        for crash in [true, false] {
            for index in 1..=trace.len() {
                let temp = TempTree::new("keyring-interrupted");
                let keyring = MockKeyring::default();
                let store = AppPrivateMeshConfigStore::with_secret_store(&temp.root, keyring.clone());
                store.save(&before).unwrap();
                let _ = boundary_run(Some((index, crash)), ||
                    std::panic::catch_unwind(|| store.join(&after.invite_code(), "ignored")));
                let restarted = AppPrivateMeshConfigStore::with_secret_store(&temp.root, keyring);
                let config = restarted.load().unwrap().unwrap();
                assert_eq!(config.node_id, before.node_id);
                assert_eq!(config.display_name, before.display_name);
                assert!((config.network_name == before.network_name && config.network_secret == before.network_secret)
                    || (config.network_name == after.network_name && config.network_secret == after.network_secret));
                for entry in fs::read_dir(&temp.root).unwrap() {
                    let bytes = fs::read(entry.unwrap().path()).unwrap();
                    for secret in [&before.network_secret, &after.network_secret] {
                        assert!(!String::from_utf8_lossy(&bytes).contains(secret.expose_secret()));
                    }
                }
            }
        }
    }

    #[test]
    fn recovery_size_limit_is_checked_before_any_persistent_write() {
        let temp = TempTree::new("recovery-large");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = store.load_or_generate("Local").unwrap();
        let metadata = encode_mesh_config_metadata(&before);
        let padding = "x".repeat(PRIVATE_ENTRY_LIMIT as usize - metadata.len() - 16);
        let metadata = format!("{metadata}ignored={padding}\n");
        write_private_file(store.config_path(), metadata.as_bytes()).unwrap();
        let original_secret = fs::read(store.secret_store.path()).unwrap();
        assert_eq!(store.load().unwrap().unwrap(), before);
        assert!(store.save(&MeshConfig::generate("Remote")).is_err());
        assert!(store.secret_store.load_recovery().unwrap().is_none());
        assert_eq!(fs::read_to_string(store.config_path()).unwrap(), metadata);
        assert_eq!(fs::read(store.secret_store.path()).unwrap(), original_secret);
        assert_eq!(store.load().unwrap().unwrap(), before);
    }

    #[test]
    fn deleted_recovery_with_failed_acknowledgement_is_explicitly_committed() {
        let temp = TempTree::new("commit-outcome");
        let keyring = MockKeyring::default();
        let store = AppPrivateMeshConfigStore::with_secret_store(&temp.root, keyring);
        let before = store.load_or_generate("Local").unwrap();
        let after = MeshConfig::generate("Remote");
        let baseline = TempTree::new("commit-outcome-baseline");
        let probe = AppPrivateMeshConfigStore::with_secret_store(&baseline.root, MockKeyring::default());
        probe.save(&before).unwrap();
        let (_, trace) = boundary_run(None, || probe.join(&after.invite_code(), "ignored").unwrap());
        let index = trace.iter().position(|step| *step == "keyring-recovery-deleted").unwrap() + 1;
        let (result, _) = boundary_run(Some((index, false)), || store.join(&after.invite_code(), "ignored"));
        assert!(matches!(result, Err(MeshStoreError::CommittedDurabilityUnconfirmed)));
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.node_id, before.node_id);
        assert_eq!(saved.display_name, before.display_name);
        assert_eq!(saved.network_name, after.network_name);
        assert_eq!(saved.network_secret, after.network_secret);
    }

    #[test]
    fn recovery_rejects_foreign_profile_and_unsafe_recovery_file() {
        let temp = TempTree::new("recovery-binding");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = store.load_or_generate("Local").unwrap();
        let after = MeshConfig::generate("Remote");
        let mut record = stage_mixed(&store, &before, &after);
        record.profile.push_str("/different-profile");
        write_private_file(&store.secret_store.recovery_path(), encode_recovery(&record).unwrap().as_bytes()).unwrap();
        let metadata = fs::read(store.config_path()).unwrap();
        assert!(store.load().is_err());
        assert!(store.join(&after.invite_code(), "ignored").is_err());
        assert_eq!(fs::read(store.config_path()).unwrap(), metadata);
        #[cfg(unix)]
        {
            fs::set_permissions(store.secret_store.recovery_path(), fs::Permissions::from_mode(0o644)).unwrap();
            assert!(store.load().is_err());
            assert!(store.delete().is_err());
            assert_eq!(fs::read(store.config_path()).unwrap(), metadata);
            assert_eq!(fs::metadata(store.secret_store.recovery_path()).unwrap().mode() & 0o777, 0o644);
        }
    }

    #[test]
    fn unsupported_secure_backend_refuses_without_downgrading_or_changing_pair() {
        struct Unsupported(MockKeyring);
        impl MeshSecretStore for Unsupported {
            fn storage_kind(&self) -> MeshSecretStorageKind { MeshSecretStorageKind::PlatformSecureStore }
            fn load_secret(&self) -> Result<Option<MeshSecret>, MeshStoreError> { self.0.load_secret() }
            fn save_secret(&self, secret: &MeshSecret) -> Result<(), MeshStoreError> { self.0.save_secret(secret) }
            fn delete_secret(&self) -> Result<(), MeshStoreError> { self.0.delete_secret() }
        }
        let temp = TempTree::new("recovery-unsupported");
        let keyring = MockKeyring::default();
        let seed = AppPrivateMeshConfigStore::with_secret_store(&temp.root, keyring.clone());
        let before = seed.load_or_generate("Original").unwrap();
        let store = AppPrivateMeshConfigStore::with_secret_store(&temp.root, Unsupported(keyring));
        let metadata = fs::read(store.config_path()).unwrap();
        assert!(store.save(&MeshConfig::generate("Remote")).is_err());
        assert_eq!(store.load().unwrap().unwrap(), before);
        assert_eq!(fs::read(store.config_path()).unwrap(), metadata);
        assert_eq!(fs::read_dir(&temp.root).unwrap().count(), if cfg!(unix) { 1 } else { 2 }); // No file credential fallback.
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_secret_temporary_is_private_and_unknown_temporary_is_retained() {
        let temp = TempTree::new("recovery-private-temporary");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let before = store.load_or_generate("Original").unwrap();
        let after = MeshConfig::generate("Remote");
        let (result, _) = boundary_run(Some((2, true)), || std::panic::catch_unwind(|| store.save(&after)));
        assert!(result.is_err());
        assert_eq!(store.load().unwrap().unwrap(), before);
        let pending: Vec<_> = fs::read_dir(&temp.root).unwrap().map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().contains(".tmp-")).collect();
        assert_eq!(pending.len(), 1);
        let path = &pending[0];
        let metadata = fs::metadata(path).unwrap();
        assert_eq!(metadata.uid(), profile_uid());
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        let retained = fs::read(path).unwrap();
        assert!(!retained.is_empty());
        assert_eq!(store.load().unwrap().unwrap(), before);
        assert_eq!(fs::read(path).unwrap(), retained); // Never sweep unknown crash leftovers.
        for entry in fs::read_dir(&temp.root).unwrap() {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            assert!(!name.contains(before.network_secret.expose_secret()));
            assert!(!name.contains(after.network_secret.expose_secret()));
        }
    }

    #[cfg(unix)]
    #[test]
    fn upgrade_reads_legacy_two_file_readonly_profile_without_adding_lock_or_widening_access() {
        let temp = TempTree::new("readonly-upgrade");
        let store = AppPrivateMeshConfigStore::new(&temp.root);
        let config = MeshConfig::generate("Legacy");
        let metadata = encode_mesh_config_metadata(&config).replace(MESH_CONFIG_METADATA_VERSION, LEGACY_METADATA_VERSION);
        write_private_file(store.config_path(), metadata.as_bytes()).unwrap();
        store.secret_store.save_secret(&config.network_secret).unwrap();
        assert_eq!(fs::read_dir(&temp.root).unwrap().count(), 2);
        fs::set_permissions(store.config_path(), fs::Permissions::from_mode(0o400)).unwrap();
        fs::set_permissions(store.secret_store.path(), fs::Permissions::from_mode(0o400)).unwrap();
        fs::set_permissions(&temp.root, fs::Permissions::from_mode(0o500)).unwrap();
        let before = (profile_snapshot(store.config_path()), profile_snapshot(store.secret_store.path()));
        assert_eq!(store.load().unwrap().unwrap(), config);
        assert_eq!(store.load_or_generate("Ignored").unwrap(), config);
        assert_eq!(before, (profile_snapshot(store.config_path()), profile_snapshot(store.secret_store.path())));
        assert_eq!(fs::read_dir(&temp.root).unwrap().count(), 2);
        assert_eq!(fs::metadata(&temp.root).unwrap().mode() & 0o777, 0o500);
        fs::set_permissions(&temp.root, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn unknown_after_credential_write_retains_record_and_refuses_forward_replay() {
        struct Unknown(MockKeyring, std::sync::atomic::AtomicBool);
        impl MeshSecretStore for Unknown {
            fn storage_kind(&self) -> MeshSecretStorageKind { MeshSecretStorageKind::PlatformSecureStore }
            fn load_secret(&self) -> Result<Option<MeshSecret>, MeshStoreError> { self.0.load_secret() }
            fn save_secret(&self, secret: &MeshSecret) -> Result<(), MeshStoreError> {
                if self.1.swap(false, Ordering::Relaxed) {
                    self.0.save_secret(&MeshSecret::generate())?;
                    return Err(transaction_error("synthetic unknown write outcome"));
                }
                self.0.save_secret(secret)
            }
            fn delete_secret(&self) -> Result<(), MeshStoreError> { self.0.delete_secret() }
            fn load_recovery(&self) -> Result<Option<MeshRecoveryRecord>, MeshStoreError> { self.0.load_recovery() }
            fn save_recovery(&self, record: &MeshRecoveryRecord) -> Result<(), MeshStoreError> { self.0.save_recovery(record) }
            fn delete_recovery(&self) -> Result<(), MeshStoreError> { self.0.delete_recovery() }
        }
        let temp = TempTree::new("unknown-write");
        let backend = Unknown(MockKeyring::default(), std::sync::atomic::AtomicBool::new(false));
        let store = AppPrivateMeshConfigStore::with_secret_store(&temp.root, backend);
        let before = store.load_or_generate("Original").unwrap();
        store.secret_store.1.store(true, Ordering::Relaxed);
        let after = MeshConfig::generate("Remote");
        assert!(matches!(store.join(&after.invite_code(), "ignored"), Err(MeshStoreError::TransactionOutcomeUnknown)));
        let retained = store.secret_store.load_recovery().unwrap();
        assert!(retained.is_some());
        assert!(store.join(&after.invite_code(), "ignored").is_err());
        assert!(store.load().is_err());
        assert_eq!(store.secret_store.load_recovery().unwrap(), retained);
        assert_eq!(fs::read_to_string(store.config_path()).unwrap(), encode_mesh_config_metadata(&before));
    }

    #[cfg(unix)]
    #[test]
    fn protected_checkpoint_helper_stages_existing_core_backward_recovery() {
        for marker in ["", "\u{001c}", "\u{001d}", "\u{001e}", "\u{001f}"] {
        let temp = TempTree::new("checkpoint-core-wire");
        let root = fs::canonicalize(&temp.root).unwrap();
        let store = AppPrivateMeshConfigStore::new(&root);
        let mut before = MeshConfig::generate("原名称");
        before.network_secret = MeshSecret::new(format!("{marker}synthetic-original-secret{marker}")).unwrap();
        store.save(&before).unwrap();
        create_private_dir(&root.join("gui-state")).unwrap();
        write_private_file(&root.join("desktop-instance.lock"), b"").unwrap();
        write_private_file(&root.join("gui-state/desktop-instance.lock"), b"").unwrap();
        let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("scripts");
        let helper = |action: &str, identifier: &str| {
            let output = std::process::Command::new("python3").args(["-B", "-c",
                "import sys,pathlib,json;sys.path.insert(0,sys.argv[1]);import mesh_profile_checkpoint as m;m.no_extra_acl=(lambda _p:None) if sys.platform=='darwin' else m.no_extra_acl;p=pathlib.Path(sys.argv[2]);expected=m.public(m.read(p/'mesh.conf'));r=m.create(p,expected) if sys.argv[3]=='create' else m.stage_restore(p,sys.argv[4],expected);print(json.dumps(r))"])
                .arg(&scripts).arg(&root).arg(action).arg(identifier).output().unwrap();
            assert!(output.status.success(), "checkpoint helper refused synthetic fixture");
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(!text.contains(before.network_secret.expose_secret()));
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        };
        let created = helper("create", "");
        let after = MeshConfig::generate("Remote");
        store.join(&after.invite_code(), "ignored").unwrap();
        let staged = helper("restore", created["checkpoint_id"].as_str().unwrap());
        assert_eq!(staged["core_startup_required"], true);
        assert_eq!(store.secret_store.load_secret().unwrap(), Some(after.network_secret));
        // On macOS only the Linux ACL query is mocked for this wire fixture;
        // it does not certify a macOS credential protection adapter.
        // The unchanged production recovery implementation consumes the helper's
        // exact wire record and restores both original values without replay.
        assert_eq!(store.load().unwrap().unwrap(), before);
        assert!(store.secret_store.load_recovery().unwrap().is_none());
        assert!(root.join("mesh.secret-checkpoint").is_file());
        }
    }

}
