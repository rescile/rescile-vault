use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use clap::{Parser, Subcommand};
use rand::RngCore;
use reqwest::blocking::Client;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use tiny_http::{Header, Method, Response, Server};
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Asset;

#[cfg(feature = "tpm")]
mod tpm;

mod update;

/// Rescile Vault: E2EE Secret Manager
#[derive(Parser, Debug, Clone)]
#[command(name = "rescile-vault", author, version, about)]
struct Cli {
    /// Rescile Vault Base URL
    #[arg(
        long,
        env = "RESCILE_VAULT_URL",
        default_value = "http://localhost:7600"
    )]
    url: String,

    /// client name
    #[arg(long, env = "RESCILE_VAULT_CLIENTNAME")]
    clientname: Option<String>,

    /// Rescile Vault Master Password
    #[arg(long, env = "RESCILE_VAULT_PASSWORD")]
    password: Option<String>,

    /// Use TPM for key protection instead of password
    #[cfg_attr(feature = "tpm", arg(long, env = "RESCILE_VAULT_TPM"))]
    #[cfg_attr(
        not(feature = "tpm"),
        arg(long, env = "RESCILE_VAULT_TPM", hide = true)
    )]
    tpm: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Manage secrets
    Secret {
        #[command(subcommand)]
        action: SecretCommands,
    },
    /// Manage collections
    Collection {
        #[command(subcommand)]
        action: CollectionCommands,
    },
    /// Manage clients
    Client {
        #[command(subcommand)]
        action: ClientCommands,
    },
    /// Run a batch of commands from a file
    Batch { file: std::path::PathBuf },
    /// Start the Web UI
    Ui,
    /// Update the application to the latest or a specific version
    Update(update::UpdateArgs),
}

#[derive(Subcommand, Debug, Clone)]
enum SecretCommands {
    List {
        collection: String,
    },
    Get {
        collection: String,
        secret_name: String,
        #[arg(long, short)]
        file: Option<std::path::PathBuf>,
    },
    Put {
        collection: String,
        secret_name: String,
        secret_value: Option<String>,
        #[arg(long, short)]
        file: Option<std::path::PathBuf>,
    },
    Delete {
        collection: String,
        secret_name: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum CollectionCommands {
    List,
    Create {
        name: String,
    },
    Invite {
        name: String,
        #[arg(long)]
        client: String,
        #[arg(long)]
        validity: Option<String>,
        #[arg(long)]
        role: Option<String>,
    },
    Delete {
        name: String,
    },
    RemoveClient {
        name: String,
        #[arg(long)]
        client: String,
    },
    Role {
        name: String,
        #[arg(long)]
        client: String,
        #[arg(long)]
        role: String,
    },
    Revoke {
        name: String,
        #[arg(long)]
        client: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ClientCommands {
    List {
        #[arg(long)]
        collection: String,
    },
    Delete {
        #[arg(long)]
        client: Option<String>,
    },
    Reset {
        #[arg(long)]
        client: String,
    },
}

#[derive(Serialize, Deserialize, Clone)]
struct KdfParams {
    #[allow(dead_code)]
    algorithm: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    initialized: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AesGcmBlob {
    nonce: String,
    ciphertext: String,
    tag: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct WrappedKey {
    ephemeral_public_key: String,
    nonce: String,
    ciphertext: String,
    tag: String,
}

#[derive(Deserialize)]
struct SessionResponse {
    token: String,
    name_salt: String,
    encrypted_private_key: AesGcmBlob,
}

#[derive(Deserialize)]
struct CipherEntry {
    id: String,
    #[allow(dead_code)]
    collection_id: String,
    blob: String,
}

#[derive(Deserialize)]
struct AccessEntry {
    #[allow(dead_code)]
    role: String,
    #[serde(flatten)]
    wrapped_key: WrappedKey,
}

#[derive(Deserialize)]
struct PendingInvite {
    #[serde(flatten)]
    blob: AesGcmBlob,
    #[allow(dead_code)]
    #[serde(default)]
    created_at: u64,
    #[allow(dead_code)]
    #[serde(default)]
    expires_at: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    role: Option<String>,
}

#[derive(Deserialize)]
struct CollectionEntry {
    collection_id: String,
    #[serde(default)]
    client_access: std::collections::HashMap<String, AccessEntry>,
    #[serde(default)]
    pending_invites: std::collections::HashMap<String, PendingInvite>,
}

#[derive(Deserialize)]
struct StateResponse {
    ciphers: Vec<CipherEntry>,
    collections: Vec<CollectionEntry>,
}

fn derive_keys(passphrase: &str, kdf: &KdfParams) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    let mut mk = [0u8; 32];
    let params = argon2::Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, None)
        .map_err(|e| format!("Argon2 params error: {}", e))?;
    let argon2 = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let salt_bytes = B64URL.decode(&kdf.salt)?;
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt_bytes, &mut mk)
        .map_err(|e| format!("Argon2 error: {}", e))?;

    let hkdf_auth = hkdf::Hkdf::<sha2::Sha256>::new(None, &mk);
    let mut ak = [0u8; 32];
    hkdf_auth
        .expand(b"rescile-vault-auth-v1", &mut ak)
        .map_err(|e| format!("HKDF expand error: {}", e))?;

    let hkdf_enc = hkdf::Hkdf::<sha2::Sha256>::new(None, &mk);
    let mut kek = [0u8; 32];
    hkdf_enc
        .expand(b"rescile-vault-enc-v1", &mut kek)
        .map_err(|e| format!("HKDF expand error: {}", e))?;

    Ok((ak.to_vec(), kek.to_vec()))
}

fn encrypt_blob(pt: &[u8], key: &[u8]) -> AesGcmBlob {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct_with_tag = cipher.encrypt(nonce, pt).unwrap();

    let ct = &ct_with_tag[..ct_with_tag.len() - 16];
    let tag = &ct_with_tag[ct_with_tag.len() - 16..];

    AesGcmBlob {
        nonce: B64URL.encode(&nonce_bytes),
        ciphertext: B64URL.encode(ct),
        tag: B64URL.encode(tag),
    }
}

fn decrypt_blob(blob: &AesGcmBlob, key: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes = B64URL.decode(&blob.nonce)?;
    let mut payload = B64URL.decode(&blob.ciphertext)?;
    payload.extend_from_slice(&B64URL.decode(&blob.tag)?);
    cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), payload.as_ref())
        .map_err(|e| format!("Decryption failed: {}", e).into())
}

fn encrypt_cipher_blob(pt: &[u8], key: &[u8]) -> String {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ct_with_tag = cipher.encrypt(Nonce::from_slice(&nonce_bytes), pt).unwrap();
    let mut full = nonce_bytes.to_vec();
    full.extend_from_slice(&ct_with_tag);
    B64URL.encode(&full)
}

fn decrypt_cipher_blob(blob: &str, key: &[u8]) -> Option<Vec<u8>> {
    let full = B64URL.decode(blob).ok()?;
    if full.len() < 28 {
        return None;
    }
    let nonce_bytes = &full[..12];
    let ct_with_tag = &full[12..];
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct_with_tag)
        .ok()
}

fn generate_random_password(len: usize) -> String {
    use rand::seq::SliceRandom;
    let upper = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let lower = b"abcdefghijklmnopqrstuvwxyz";
    let digits = b"0123456789";
    let special = b"!@#$%^&*";
    let all = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*";
    let mut rng = rand::thread_rng();
    let effective_len = if len < 4 { 16 } else { len };

    let mut password: Vec<u8> = Vec::with_capacity(effective_len);
    password.push(*upper.choose(&mut rng).unwrap());
    password.push(*lower.choose(&mut rng).unwrap());
    password.push(*digits.choose(&mut rng).unwrap());
    password.push(*special.choose(&mut rng).unwrap());
    for _ in 4..effective_len {
        password.push(*all.choose(&mut rng).unwrap());
    }
    password.as_mut_slice().shuffle(&mut rng);
    String::from_utf8(password).unwrap()
}

fn url_encode(input: &str) -> String {
    input
        .as_bytes()
        .iter()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (*b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in line.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn parse_validity(validity: &str) -> Result<u64, Box<dyn Error>> {
    let duration = humantime::parse_duration(validity)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Ok(now + duration.as_secs())
}

fn get_default_clientname() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| {
            if let Ok(content) = std::fs::read_to_string("/etc/hostname") {
                content.trim().to_string()
            } else {
                "localhost".to_string()
            }
        });

    if user == "root" || user == "Administrator" {
        host
    } else {
        format!("{}@{}", user, host)
    }
}

fn get_default_password() -> Result<String, Box<dyn Error>> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let is_root = user == "root" || user == "Administrator";

    let path = if is_root {
        std::path::PathBuf::from("/etc/rescile/vault.password")
    } else if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        std::path::PathBuf::from(val).join("rescile/vault.password")
    } else if let Ok(val) = std::env::var("APPDATA") {
        std::path::PathBuf::from(val).join("rescile/vault.password")
    } else if let Ok(val) = std::env::var("HOME") {
        if cfg!(target_os = "macos") {
            std::path::PathBuf::from(val).join("Library/Application Support/rescile/vault.password")
        } else {
            std::path::PathBuf::from(val).join(".config/rescile/vault.password")
        }
    } else {
        return Err("Cannot determine home directory for password file".into());
    };

    if path.exists() {
        let pass = std::fs::read_to_string(&path)?.trim().to_string();
        Ok(pass)
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let new_pass = generate_random_password(64);
        std::fs::write(&path, &new_pass)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(&path, perms);
            }
        }
        Ok(new_pass)
    }
}

struct VaultContext {
    #[allow(dead_code)]
    cli: Cli,
    clientname: String,
    password: Option<String>,
    client: Client,
    base_url: String,
    session: Option<SessionResponse>,
    static_secret: Option<StaticSecret>,
    collection_keys: HashMap<String, [u8; 32]>,
}

impl VaultContext {
    fn new(cli: Cli) -> Self {
        let base_url = cli.url.trim_end_matches('/').to_string();

        let clientname = cli
            .clientname
            .clone()
            .unwrap_or_else(get_default_clientname);

        #[cfg(feature = "tpm")]
        let use_tpm = cli.tpm;
        #[cfg(not(feature = "tpm"))]
        let use_tpm = false;

        let password = if cli.password.is_none() && !use_tpm {
            get_default_password().ok()
        } else {
            cli.password.clone()
        };

        Self {
            cli,
            clientname,
            password,
            client: Client::new(),
            base_url,
            session: None,
            static_secret: None,
            collection_keys: HashMap::new(),
        }
    }

    fn derive_master_keys(&self) -> Result<(Vec<u8>, Vec<u8>, KdfParams), Box<dyn Error>> {
        #[cfg(feature = "tpm")]
        if self.cli.tpm {
            // For TPM mode we still need KDF params for registration, but the password
            // is not used. We generate dummy KDF params with a random salt.
            let mut salt_bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut salt_bytes);
            let kdf_params = KdfParams {
                algorithm: "argon2id".to_string(),
                memory_kib: 262144,
                iterations: 3,
                parallelism: 4,
                salt: B64URL.encode(&salt_bytes),
                initialized: true,
            };
            let (ak, kek) = tpm::tpm_derive_keys(&self.clientname)?;
            return Ok((ak, kek, kdf_params));
        }
        let password = self
            .password
            .as_deref()
            .ok_or("Password must be provided or generated")?;
        self.derive_password_keys(password)
    }

    fn derive_password_keys(
        &self,
        password: &str,
    ) -> Result<(Vec<u8>, Vec<u8>, KdfParams), Box<dyn Error>> {
        let kdf_url = format!("{}/vault/v1/kdf/{}", self.base_url, self.clientname);
        let kdf_req = self.client.get(&kdf_url).send()?;

        let kdf_params = if kdf_req.status() == reqwest::StatusCode::NOT_FOUND {
            let mut salt_bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut salt_bytes);
            KdfParams {
                algorithm: "argon2id".to_string(),
                memory_kib: 262144,
                iterations: 3,
                parallelism: 4,
                salt: B64URL.encode(&salt_bytes),
                initialized: true,
            }
        } else {
            kdf_req.json()?
        };

        let (ak, kek) = derive_keys(password, &kdf_params)?;
        Ok((ak, kek, kdf_params))
    }

    fn authenticate(&mut self) -> Result<(), Box<dyn Error>> {
        if self.session.is_some() {
            return Ok(());
        }

        #[cfg(feature = "tpm")]
        let use_tpm = self.cli.tpm;
        #[cfg(not(feature = "tpm"))]
        let use_tpm = false;

        let kdf_url = format!("{}/vault/v1/kdf/{}", self.base_url, self.clientname);
        let kdf_req = self.client.get(&kdf_url).send()?;

        if kdf_req.status() == reqwest::StatusCode::NOT_FOUND {
            // New client registration
            let (ak, kek, kdf_params) = self.derive_master_keys()?;

            let mut rng = rand::thread_rng();
            let static_secret = StaticSecret::random_from_rng(&mut rng);
            let public_key = PublicKey::from(&static_secret);

            let encrypted_private_key = encrypt_blob(&static_secret.to_bytes(), &kek);

            let resp = self
                .client
                .post(format!("{}/vault/v1/register", self.base_url))
                .json(&serde_json::json!({
                    "client_id": self.clientname,
                    "kdf_params": kdf_params,
                    "auth_key": B64URL.encode(&ak),
                    "public_key": B64URL.encode(public_key.as_bytes()),
                    "encrypted_private_key": encrypted_private_key
                }))
                .send()?;

            if !resp.status().is_success() {
                return Err(format!("Failed to register client: {}", resp.text()?).into());
            }
            // Now authenticate with the same keys
            let session_resp = self
                .client
                .post(format!("{}/vault/v1/session", self.base_url))
                .json(&serde_json::json!({
                    "auth_key": B64URL.encode(&ak),
                    "client_id": self.clientname
                }))
                .send()?;

            if !session_resp.status().is_success() {
                let invite_token_b64 =
                    std::env::var("RESCILE_VAULT_INVITE_TOKEN").unwrap_or_else(|_| String::new());
                let invite_token_b64 = if invite_token_b64.is_empty() {
                    let resp = self
                        .client
                        .get(format!(
                            "{}/vault/v1/invite/{}",
                            self.base_url,
                            url_encode(&self.clientname)
                        ))
                        .send()?;
                    if resp.status().is_success() {
                        resp.text()?
                    } else {
                        String::new()
                    }
                } else {
                    invite_token_b64
                };

                if !invite_token_b64.is_empty() {
                    let mut rng = rand::thread_rng();
                    let static_secret = StaticSecret::random_from_rng(&mut rng);
                    let public_key = PublicKey::from(&static_secret);
                    let encrypted_private_key = encrypt_blob(&static_secret.to_bytes(), &kek);

                    let reset_resp = self
                        .client
                        .post(format!(
                            "{}/vault/v1/client/{}/reset",
                            self.base_url,
                            url_encode(&self.clientname)
                        ))
                        .json(&serde_json::json!({
                            "token": invite_token_b64,
                            "kdf_params": kdf_params,
                            "auth_key": B64URL.encode(&ak),
                            "public_key": B64URL.encode(public_key.as_bytes()),
                            "encrypted_private_key": encrypted_private_key
                        }))
                        .send()?;

                    if reset_resp.status().is_success() {
                        let session_resp = self.client.post(format!("{}/vault/v1/session", self.base_url))
                            .json(&serde_json::json!({"auth_key": B64URL.encode(&ak), "client_id": self.clientname})).send()?;
                        if session_resp.status().is_success() {
                            let session: SessionResponse = session_resp.json()?;
                            self.session = Some(session);
                            self.static_secret = Some(static_secret);
                            return Ok(());
                        }
                    }
                }
                return Err(format!("Authentication failed: {}", session_resp.text()?).into());
            }

            let session: SessionResponse = session_resp.json()?;
            let priv_key_bytes = decrypt_blob(&session.encrypted_private_key, &kek)?;
            let static_secret =
                StaticSecret::from(<[u8; 32]>::try_from(priv_key_bytes.as_slice()).unwrap());

            self.session = Some(session);
            self.static_secret = Some(static_secret);
        } else {
            // Existing client
            let kdf_params: KdfParams = kdf_req.json()?;

            let (ak, kek) = if use_tpm {
                #[cfg(feature = "tpm")]
                {
                    let (ak, kek) = tpm::tpm_derive_keys(&self.clientname)?;
                    (ak, kek)
                }
                #[cfg(not(feature = "tpm"))]
                {
                    unreachable!()
                }
            } else {
                let password = self.password.as_deref().unwrap();
                derive_keys(password, &kdf_params)?
            };

            let session_resp = self
                .client
                .post(format!("{}/vault/v1/session", self.base_url))
                .json(&serde_json::json!({
                    "auth_key": B64URL.encode(&ak),
                     "client_id": self.clientname
                }))
                .send()?;

            if !session_resp.status().is_success() {
                return Err(format!("Authentication failed: {}", session_resp.text()?).into());
            }

            let session: SessionResponse = session_resp.json()?;
            let priv_key_bytes = decrypt_blob(&session.encrypted_private_key, &kek)?;
            let static_secret =
                StaticSecret::from(<[u8; 32]>::try_from(priv_key_bytes.as_slice()).unwrap());

            self.session = Some(session);
            self.static_secret = Some(static_secret);
        }

        Ok(())
    }

    fn get_state(&self) -> Result<StateResponse, Box<dyn Error>> {
        let state: StateResponse = self
            .client
            .get(format!("{}/vault/v1/state", self.base_url))
            .bearer_auth(&self.session.as_ref().unwrap().token)
            .send()?
            .json()?;
        Ok(state)
    }

    fn resolve_collection_key(
        &mut self,
        collection_name: &str,
        state: &mut StateResponse,
    ) -> Result<[u8; 32], Box<dyn Error>> {
        if let Some(k) = self.collection_keys.get(collection_name) {
            return Ok(*k);
        }

        let static_secret = self.static_secret.as_ref().unwrap();

        if let Some(col) = state
            .collections
            .iter()
            .find(|c| c.collection_id == collection_name)
        {
            if let Some(access) = col.client_access.get(&self.clientname) {
                let wrapped_key = &access.wrapped_key;
                let ephemeral_public_bytes =
                    B64URL.decode(&wrapped_key.ephemeral_public_key).unwrap();
                let ephemeral_public = PublicKey::from(
                    <[u8; 32]>::try_from(ephemeral_public_bytes.as_slice()).unwrap(),
                );
                let shared_secret = static_secret.diffie_hellman(&ephemeral_public);
                let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, shared_secret.as_bytes());
                let mut wrap_key = [0u8; 32];
                hkdf.expand(b"rescile-collection-wrap-v1", &mut wrap_key)
                    .unwrap();

                let wrapped_blob = AesGcmBlob {
                    nonce: wrapped_key.nonce.clone(),
                    ciphertext: wrapped_key.ciphertext.clone(),
                    tag: wrapped_key.tag.clone(),
                };
                let decrypted_ck = decrypt_blob(&wrapped_blob, &wrap_key)?;
                let mut collection_key = [0u8; 32];
                collection_key.copy_from_slice(&decrypted_ck);
                self.collection_keys
                    .insert(collection_name.to_string(), collection_key);
                return Ok(collection_key);
            } else if let Some(invite) = col.pending_invites.get(&self.clientname) {
                let invite_token_b64 =
                    std::env::var("RESCILE_VAULT_INVITE_TOKEN").unwrap_or_else(|_| String::new());
                let invite_token_b64 = if invite_token_b64.is_empty() {
                    let resp = self
                        .client
                        .get(format!(
                            "{}/vault/v1/invite/{}",
                            self.base_url,
                            url_encode(&self.clientname)
                        ))
                        .send()?;
                    if !resp.status().is_success() {
                        return Err(
                            "Missing RESCILE_VAULT_INVITE_TOKEN and no token found on server"
                                .into(),
                        );
                    }
                    resp.text()?
                } else {
                    invite_token_b64
                };
                let invite_token = B64URL.decode(&invite_token_b64)?;
                if invite_token.len() != 32 {
                    return Err("Invalid invite token length".into());
                }
                let collection_key_vec = decrypt_blob(&invite.blob, &invite_token)?;
                let mut collection_key = [0u8; 32];
                collection_key.copy_from_slice(&collection_key_vec);

                let mut rng = rand::thread_rng();
                let ephemeral_secret = EphemeralSecret::random_from_rng(&mut rng);
                let ephemeral_public = PublicKey::from(&ephemeral_secret);
                let recipient_public = PublicKey::from(static_secret);
                let shared_secret = ephemeral_secret.diffie_hellman(&recipient_public);
                let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, shared_secret.as_bytes());
                let mut wrap_key = [0u8; 32];
                hkdf.expand(b"rescile-collection-wrap-v1", &mut wrap_key)
                    .unwrap();
                let wrapped_blob = encrypt_blob(&collection_key, &wrap_key);
                let wrapped_key = WrappedKey {
                    ephemeral_public_key: B64URL.encode(ephemeral_public.as_bytes()),
                    nonce: wrapped_blob.nonce,
                    ciphertext: wrapped_blob.ciphertext,
                    tag: wrapped_blob.tag,
                };

                let resp = self
                    .client
                    .post(format!(
                        "{}/vault/v1/collection/{}/claim",
                        self.base_url, collection_name
                    ))
                    .bearer_auth(&self.session.as_ref().unwrap().token)
                    .json(&serde_json::json!({
                        "wrapped_key": wrapped_key
                    }))
                    .send()?;

                if !resp.status().is_success() {
                    return Err(format!("Failed to claim invite: {}", resp.text()?).into());
                }

                self.collection_keys
                    .insert(collection_name.to_string(), collection_key);
                return Ok(collection_key);
            } else {
                return Err(
                    format!("You don't have access to collection '{}'", collection_name).into(),
                );
            }
        } else {
            if collection_name == "default" {
                let mut collection_key = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut collection_key);

                let mut rng = rand::thread_rng();
                let ephemeral_secret = EphemeralSecret::random_from_rng(&mut rng);
                let ephemeral_public = PublicKey::from(&ephemeral_secret);
                let recipient_public = PublicKey::from(static_secret);
                let shared_secret = ephemeral_secret.diffie_hellman(&recipient_public);
                let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, shared_secret.as_bytes());
                let mut wrap_key = [0u8; 32];
                hkdf.expand(b"rescile-collection-wrap-v1", &mut wrap_key)
                    .unwrap();
                let wrapped_blob = encrypt_blob(&collection_key, &wrap_key);
                let wrapped_key = WrappedKey {
                    ephemeral_public_key: B64URL.encode(ephemeral_public.as_bytes()),
                    nonce: wrapped_blob.nonce,
                    ciphertext: wrapped_blob.ciphertext,
                    tag: wrapped_blob.tag,
                };

                let col_resp = self
                    .client
                    .post(format!("{}/vault/v1/collection", self.base_url))
                    .bearer_auth(&self.session.as_ref().unwrap().token)
                    .json(&serde_json::json!({
                        "collection_id": "default",
                        "wrapped_key": wrapped_key
                    }))
                    .send()?;

                if !col_resp.status().is_success() {
                    return Err(format!(
                        "Failed to create default collection: {}",
                        col_resp.text()?
                    )
                    .into());
                }

                self.collection_keys
                    .insert("default".to_string(), collection_key);
                return Ok(collection_key);
            } else {
                return Err(format!(
                    "Collection '{}' not found or you don't have access to it",
                    collection_name
                )
                .into());
            }
        }
    }

    fn create_collection(&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        let state = self.get_state()?;
        if state.collections.iter().any(|c| c.collection_id == name) {
            return Err("Collection already exists".into());
        }

        let mut collection_key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut collection_key);

        let static_secret = self.static_secret.as_ref().unwrap();
        let mut rng = rand::thread_rng();
        let ephemeral_secret = EphemeralSecret::random_from_rng(&mut rng);
        let ephemeral_public = PublicKey::from(&ephemeral_secret);
        let recipient_public = PublicKey::from(static_secret);
        let shared_secret = ephemeral_secret.diffie_hellman(&recipient_public);
        let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, shared_secret.as_bytes());
        let mut wrap_key = [0u8; 32];
        hkdf.expand(b"rescile-collection-wrap-v1", &mut wrap_key)
            .unwrap();
        let wrapped_blob = encrypt_blob(&collection_key, &wrap_key);
        let wrapped_key = WrappedKey {
            ephemeral_public_key: B64URL.encode(ephemeral_public.as_bytes()),
            nonce: wrapped_blob.nonce,
            ciphertext: wrapped_blob.ciphertext,
            tag: wrapped_blob.tag,
        };

        let resp = self
            .client
            .post(format!("{}/vault/v1/collection", self.base_url))
            .bearer_auth(&self.session.as_ref().unwrap().token)
            .json(&serde_json::json!({
                "collection_id": name,
                "wrapped_key": wrapped_key
            }))
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to create collection: {}", resp.text()?).into());
        }

        self.collection_keys
            .insert(name.to_string(), collection_key);
        Ok(())
    }

    fn api_invite_client(
        &mut self,
        collection_name: &str,
        target_client: &str,
        validity: Option<String>,
        role: Option<String>,
    ) -> Result<String, Box<dyn Error>> {
        let mut state = self.get_state()?;
        let collection_key = self.resolve_collection_key(collection_name, &mut state)?;

        let expires_at = match validity {
            Some(v) => Some(parse_validity(&v)?),
            None => None,
        };

        let token_resp = self
            .client
            .get(format!(
                "{}/vault/v1/invite/{}",
                self.base_url,
                url_encode(target_client)
            ))
            .send()?;

        let mut invite_secret = [0u8; 32];
        let token_b64 = if token_resp.status().is_success() {
            let text = token_resp.text()?;
            if let Ok(decoded) = B64URL.decode(&text) {
                if decoded.len() == 32 {
                    invite_secret.copy_from_slice(&decoded);
                    text
                } else {
                    rand::thread_rng().fill_bytes(&mut invite_secret);
                    B64URL.encode(&invite_secret)
                }
            } else {
                rand::thread_rng().fill_bytes(&mut invite_secret);
                B64URL.encode(&invite_secret)
            }
        } else {
            rand::thread_rng().fill_bytes(&mut invite_secret);
            B64URL.encode(&invite_secret)
        };

        let blob = encrypt_blob(&collection_key, &invite_secret);

        let resp = self
            .client
            .post(format!(
                "{}/vault/v1/collection/{}/invite",
                self.base_url, collection_name
            ))
            .bearer_auth(&self.session.as_ref().unwrap().token)
            .json(&serde_json::json!({
                "client_id": target_client,
                "blob": blob,
                "token": token_b64,
                "expires_at": expires_at,
                "role": role,
            }))
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to invite client: {}", resp.text()?).into());
        }

        Ok(token_b64)
    }

    fn invite_client(
        &mut self,
        collection_name: &str,
        target_client: &str,
        validity: Option<String>,
        role: Option<String>,
    ) -> Result<(), Box<dyn Error>> {
        let token = self.api_invite_client(collection_name, target_client, validity, role)?;
        println!("{}", token);
        Ok(())
    }

    fn cmd_get(
        &mut self,
        collection: &str,
        secret_name: &str,
        file: Option<&std::path::PathBuf>,
    ) -> Result<(), Box<dyn Error>> {
        let (bytes, _) = self.api_get_secret_bytes(collection, secret_name)?;
        if let Some(path) = file {
            std::fs::write(path, bytes)?;
        } else {
            print!("{}", String::from_utf8_lossy(&bytes).trim_end());
        }

        Ok(())
    }

    fn cmd_put(
        &mut self,
        collection: &str,
        secret_name: &str,
        secret_value: Option<String>,
        file: Option<&std::path::PathBuf>,
    ) -> Result<(), Box<dyn Error>> {
        let final_value = if let Some(path) = file {
            Some(std::fs::read(path)?)
        } else {
            secret_value.map(|s| s.into_bytes())
        };

        self.api_put_secret_bytes(collection, secret_name, final_value)?;
        Ok(())
    }

    fn cmd_delete_secret(
        &mut self,
        collection: &str,
        secret_name: &str,
    ) -> Result<(), Box<dyn Error>> {
        let session = self.session.as_ref().unwrap();
        let name_salt = B64URL.decode(&session.name_salt).unwrap();
        let mut hasher = blake3::Hasher::new();
        hasher.update(collection.as_bytes());
        hasher.update(secret_name.as_bytes());
        hasher.update(&name_salt);
        let id = hasher.finalize().to_string();

        let resp = self
            .client
            .delete(format!("{}/vault/v1/cipher/{}", self.base_url, id))
            .bearer_auth(&session.token)
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to delete secret: {}", resp.text()?).into());
        }
        Ok(())
    }

    fn cmd_delete_collection(&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        let session = self.session.as_ref().unwrap();
        let resp = self
            .client
            .delete(format!("{}/vault/v1/collection/{}", self.base_url, name))
            .bearer_auth(&session.token)
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to delete collection: {}", resp.text()?).into());
        }
        Ok(())
    }

    fn cmd_remove_client(&mut self, name: &str, client: &str) -> Result<(), Box<dyn Error>> {
        let session = self.session.as_ref().unwrap();
        let resp = self
            .client
            .delete(format!(
                "{}/vault/v1/collection/{}/client/{}",
                self.base_url, name, client
            ))
            .bearer_auth(&session.token)
            .send()?;

        if !resp.status().is_success() {
            return Err(
                format!("Failed to remove client from collection: {}", resp.text()?).into(),
            );
        }
        Ok(())
    }

    fn cmd_collection_role(
        &mut self,
        name: &str,
        client: &str,
        role: &str,
    ) -> Result<(), Box<dyn Error>> {
        let session = self.session.as_ref().unwrap();
        let valid_roles = ["Owner", "Member", "Reader"];
        if !valid_roles.contains(&role) {
            return Err("Invalid role. Must be Owner, Member, or Reader".into());
        }

        let resp = self
            .client
            .put(format!(
                "{}/vault/v1/collection/{}/client/{}/role",
                self.base_url, name, client
            ))
            .bearer_auth(&session.token)
            .json(&serde_json::json!({ "role": role }))
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to update role: {}", resp.text()?).into());
        }
        Ok(())
    }

    fn cmd_collection_revoke(&mut self, name: &str, client: &str) -> Result<(), Box<dyn Error>> {
        let session = self.session.as_ref().unwrap();
        let resp = self
            .client
            .delete(format!(
                "{}/vault/v1/collection/{}/invite/{}",
                self.base_url, name, client
            ))
            .bearer_auth(&session.token)
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to revoke invite: {}", resp.text()?).into());
        }
        Ok(())
    }

    fn cmd_list_collections(&self) -> Result<(), Box<dyn Error>> {
        let state = self.get_state()?;
        for col in state.collections {
            println!("{}", col.collection_id);
        }
        Ok(())
    }

    fn cmd_list_secrets(&self, collection: &str) -> Result<(), Box<dyn Error>> {
        let state = self.get_state()?;
        for c in state.ciphers {
            if c.collection_id == collection {
                println!("{}", c.id);
            }
        }
        Ok(())
    }

    fn cmd_list_clients(&self, collection: &str) -> Result<(), Box<dyn Error>> {
        let state = self.get_state()?;
        if let Some(col) = state
            .collections
            .iter()
            .find(|c| c.collection_id == collection)
        {
            for client in col.client_access.keys() {
                println!("{}\tenrolled", client);
            }
            for client in col.pending_invites.keys() {
                println!("{}\tinvited", client);
            }
        } else {
            return Err(format!(
                "Collection '{}' not found or you don't have access to it",
                collection
            )
            .into());
        }
        Ok(())
    }

    fn cmd_delete_client(&mut self, target_client: Option<&str>) -> Result<(), Box<dyn Error>> {
        let session = self.session.as_ref().unwrap();
        let client_to_delete = target_client.unwrap_or(&self.clientname);
        let resp = self
            .client
            .delete(format!(
                "{}/vault/v1/client/{}",
                self.base_url,
                url_encode(client_to_delete)
            ))
            .bearer_auth(&session.token)
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to delete client: {}", resp.text()?).into());
        }
        Ok(())
    }

    fn api_get_secret_bytes(
        &mut self,
        collection: &str,
        secret_name: &str,
    ) -> Result<(Vec<u8>, bool), Box<dyn Error>> {
        let mut state = self.get_state()?;
        let collection_key = self.resolve_collection_key(collection, &mut state)?;

        let session = self.session.as_ref().unwrap();
        let name_salt = B64URL.decode(&session.name_salt).unwrap();
        let mut hasher = blake3::Hasher::new();
        hasher.update(collection.as_bytes());
        hasher.update(secret_name.as_bytes());
        hasher.update(&name_salt);
        let id = hasher.finalize().to_string();

        if let Some(cipher) = state.ciphers.iter().find(|c| c.id == id) {
            let pt = decrypt_cipher_blob(&cipher.blob, &collection_key)
                .ok_or("Failed to decrypt cipher")?;
            return Ok((pt, false));
        }

        let new_password = generate_random_password(32).into_bytes();
        let new_blob = encrypt_cipher_blob(&new_password, &collection_key);

        let put_resp = self
            .client
            .put(format!("{}/vault/v1/cipher/{}", self.base_url, id))
            .bearer_auth(&session.token)
            .json(&serde_json::json!({
                "collection_id": collection,
                "blob": new_blob,
                "updated_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
            }))
            .send()?;

        if !put_resp.status().is_success() {
            return Err(format!("Failed to save new cipher: {}", put_resp.text()?).into());
        }

        Ok((new_password, true))
    }

    fn api_put_secret_bytes(
        &mut self,
        collection: &str,
        secret_name: &str,
        secret_value: Option<Vec<u8>>,
    ) -> Result<(Vec<u8>, bool), Box<dyn Error>> {
        let mut state = self.get_state()?;
        let collection_key = self.resolve_collection_key(collection, &mut state)?;

        let session = self.session.as_ref().unwrap();
        let name_salt = B64URL.decode(&session.name_salt).unwrap();
        let mut hasher = blake3::Hasher::new();
        hasher.update(collection.as_bytes());
        hasher.update(secret_name.as_bytes());
        hasher.update(&name_salt);
        let id = hasher.finalize().to_string();

        let (final_value, generated) = match secret_value {
            Some(v) if !v.is_empty() => (v, false),
            _ => (generate_random_password(32).into_bytes(), true),
        };

        if final_value.len() > 1024 * 1024 {
            return Err("Secret exceeds the maximum allowed size of 1 Megabyte.".into());
        }

        let new_blob = encrypt_cipher_blob(&final_value, &collection_key);

        let put_resp = self
            .client
            .put(format!("{}/vault/v1/cipher/{}", self.base_url, id))
            .bearer_auth(&session.token)
            .json(&serde_json::json!({
                "collection_id": collection,
                "blob": new_blob,
                "updated_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
            }))
            .send()?;

        if !put_resp.status().is_success() {
            if put_resp.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
                return Err("Secret exceeds the maximum allowed size of 1 Megabyte.".into());
            }
            return Err(format!("Failed to save cipher: {}", put_resp.text()?).into());
        }

        Ok((final_value, generated))
    }

    fn api_batch(
        &mut self,
        collection: &str,
        create_if_missing: bool,
        secrets: &[(String, Option<String>)],
        invites: &[(String, Option<String>, Option<String>)],
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let state = self.get_state()?;
        let collection_existed = state
            .collections
            .iter()
            .any(|c| c.collection_id == collection);

        let mut collection_created = false;
        if !collection_existed {
            if !create_if_missing {
                return Err(format!(
                    "Collection '{}' does not exist or you don't have access to it",
                    collection
                )
                .into());
            }
            self.create_collection(collection)?;
            collection_created = true;
        }

        // Precompute existence of each requested secret to classify the per-item status.
        let state = self.get_state()?;
        let name_salt = B64URL
            .decode(&self.session.as_ref().unwrap().name_salt)
            .unwrap();
        let secret_exists: Vec<bool> = secrets
            .iter()
            .map(|(name, _)| {
                if name.is_empty() {
                    return false;
                }
                let mut hasher = blake3::Hasher::new();
                hasher.update(collection.as_bytes());
                hasher.update(name.as_bytes());
                hasher.update(&name_salt);
                let id = hasher.finalize().to_string();
                state.ciphers.iter().any(|c| c.id == id)
            })
            .collect();
        drop(state);

        let mut secret_results: Vec<serde_json::Value> = Vec::with_capacity(secrets.len());
        for ((name, value), existed) in secrets.iter().zip(secret_exists.iter()) {
            if name.is_empty() {
                secret_results.push(serde_json::json!({
                    "name": name,
                    "status": "error",
                    "error": "missing name",
                }));
                continue;
            }
            let supplied = value.as_deref().map(|v| !v.is_empty()).unwrap_or(false);
            if supplied {
                match self.api_put_secret_bytes(
                    collection,
                    name,
                    value.clone().map(|s| s.into_bytes()),
                ) {
                    Ok(_) => secret_results.push(serde_json::json!({
                        "name": name,
                        "status": if *existed { "updated" } else { "created" },
                        "generated": false,
                    })),
                    Err(e) => secret_results.push(serde_json::json!({
                        "name": name,
                        "status": "error",
                        "error": format!("{}", e),
                    })),
                }
            } else if *existed {
                secret_results.push(serde_json::json!({
                    "name": name,
                    "status": "kept",
                    "generated": false,
                }));
            } else {
                match self.api_put_secret_bytes(collection, name, None) {
                    Ok((generated_value, _)) => secret_results.push(serde_json::json!({
                        "name": name,
                        "status": "created",
                        "generated": true,
                        "value": String::from_utf8_lossy(&generated_value).into_owned(),
                    })),
                    Err(e) => secret_results.push(serde_json::json!({
                        "name": name,
                        "status": "error",
                        "error": format!("{}", e),
                    })),
                }
            }
        }

        let mut invite_results: Vec<serde_json::Value> = Vec::with_capacity(invites.len());
        for (client, validity, role) in invites {
            if client.is_empty() {
                invite_results.push(serde_json::json!({
                    "client": client,
                    "status": "error",
                    "error": "missing client",
                }));
                continue;
            }
            match self.api_invite_client(collection, client, validity.clone(), role.clone()) {
                Ok(token) => invite_results.push(serde_json::json!({
                    "client": client,
                    "status": "invited",
                    "token": token,
                })),
                Err(e) => invite_results.push(serde_json::json!({
                    "client": client,
                    "status": "error",
                    "error": format!("{}", e),
                })),
            }
        }

        Ok(serde_json::json!({
            "collection": collection,
            "collection_created": collection_created,
            "collection_existed": collection_existed,
            "secrets": secret_results,
            "invites": invite_results,
        }))
    }

    fn cmd_ui(&self) -> Result<(), Box<dyn Error>> {
        use std::sync::Mutex;

        let initial_url = self.cli.url.clone();
        let initial_clientname = self.clientname.clone();
        let initial_password = self.password.clone();
        let url_set = std::env::var("RESCILE_VAULT_URL").is_ok();
        let clientname_set = self.cli.clientname.is_some();
        let password_set = self.cli.password.is_some();
        let tpm_flag = self.cli.tpm;
        let credentials_provided = (password_set || tpm_flag) && clientname_set;

        let ctx_lock: Mutex<Option<VaultContext>> = Mutex::new(None);

        if credentials_provided {
            let auto_cli = Cli {
                url: initial_url.clone(),
                clientname: Some(initial_clientname.clone()),
                password: initial_password.clone(),
                tpm: tpm_flag,
                command: Commands::Ui,
            };
            let mut ctx = VaultContext::new(auto_cli);
            if ctx.authenticate().is_ok() {
                *ctx_lock.lock().unwrap() = Some(ctx);
            }
        }

        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        println!("Web UI started at http://127.0.0.1:{}", port);

        let json_response = |status: u16, value: serde_json::Value| {
            let body = serde_json::to_vec(&value).unwrap();
            Response::from_data(body)
                .with_status_code(status)
                .with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                )
        };

        for mut request in server.incoming_requests() {
            let full_url = request.url().to_string();
            let method = request.method().clone();
            let path_only = full_url.split('?').next().unwrap_or("/").to_string();

            if path_only.starts_with("/api/") {
                let mut body_str = String::new();
                if matches!(method, Method::Post | Method::Put) {
                    let _ = request.as_reader().read_to_string(&mut body_str);
                }
                let body: serde_json::Value = if body_str.is_empty() {
                    serde_json::Value::Null
                } else {
                    match serde_json::from_str(&body_str) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": format!("Invalid JSON: {}", e)}),
                            ));
                            continue;
                        }
                    }
                };

                let str_field = |key: &str| -> Option<String> {
                    body.get(key).and_then(|v| v.as_str()).map(String::from)
                };

                match (&method, path_only.as_str()) {
                    (Method::Get, "/api/config") => {
                        let guard = ctx_lock.lock().unwrap();
                        let (ready, current_clientname, current_url) = match guard.as_ref() {
                            Some(ctx) => (true, ctx.clientname.clone(), ctx.base_url.clone()),
                            None => (false, initial_clientname.clone(), initial_url.clone()),
                        };
                        drop(guard);
                        let value = serde_json::json!({
                            "url": current_url,
                            "clientname": current_clientname,
                            "has_url": url_set,
                            "has_clientname": clientname_set,
                            "has_password": password_set || tpm_flag,
                            "ready": ready,
                        });
                        let _ = request.respond(json_response(200, value));
                    }
                    (Method::Post, "/api/login") => {
                        let url = str_field("url").unwrap_or_else(|| initial_url.clone());
                        let clientname =
                            str_field("clientname").unwrap_or_else(|| initial_clientname.clone());
                        let password = str_field("password").or_else(|| initial_password.clone());

                        if !tpm_flag && password.as_deref().map(|p| p.is_empty()).unwrap_or(true) {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": "Password is required"}),
                            ));
                            continue;
                        }

                        let cli_for_login = Cli {
                            url,
                            clientname: Some(clientname),
                            password,
                            tpm: tpm_flag,
                            command: Commands::Ui,
                        };
                        let mut ctx = VaultContext::new(cli_for_login);
                        match ctx.authenticate() {
                            Ok(_) => {
                                *ctx_lock.lock().unwrap() = Some(ctx);
                                let _ = request
                                    .respond(json_response(200, serde_json::json!({"ok": true})));
                            }
                            Err(e) => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": format!("{}", e)}),
                                ));
                            }
                        }
                    }
                    (Method::Post, "/api/logout") => {
                        *ctx_lock.lock().unwrap() = None;
                        let _ =
                            request.respond(json_response(200, serde_json::json!({"ok": true})));
                    }
                    (Method::Get, "/api/state") => {
                        let mut guard = ctx_lock.lock().unwrap();
                        match guard.as_mut() {
                            None => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": "Not authenticated"}),
                                ));
                            }
                            Some(ctx) => match ctx.get_state() {
                                Ok(state) => {
                                    let clientname = ctx.clientname.clone();
                                    let collections: Vec<serde_json::Value> = state
                                        .collections
                                        .iter()
                                        .filter(|c| {
                                            c.client_access.contains_key(&clientname)
                                                || c.pending_invites.contains_key(&clientname)
                                        })
                                        .map(|c| {
                                            let mut members: Vec<&String> =
                                                c.client_access.keys().collect();
                                            members.sort();
                                            let mut pending_invitees: Vec<&String> =
                                                c.pending_invites.keys().collect();
                                            pending_invitees.sort();
                                            serde_json::json!({
                                                "name": c.collection_id,
                                                "has_access": c.client_access.contains_key(&clientname),
                                                "pending_invite": c.pending_invites.contains_key(&clientname),
                                                "members": members,
                                                "pending_invitees": pending_invitees,
                                            })
                                        })
                                        .collect();
                                    let _ = request.respond(json_response(
                                        200,
                                        serde_json::json!({
                                            "clientname": clientname,
                                            "collections": collections,
                                        }),
                                    ));
                                }
                                Err(e) => {
                                    let _ = request.respond(json_response(
                                        500,
                                        serde_json::json!({"error": format!("{}", e)}),
                                    ));
                                }
                            },
                        }
                    }
                    (Method::Post, "/api/collection") => {
                        let name = str_field("name").unwrap_or_default();
                        if name.is_empty() {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": "Collection name is required"}),
                            ));
                            continue;
                        }
                        let mut guard = ctx_lock.lock().unwrap();
                        match guard.as_mut() {
                            None => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": "Not authenticated"}),
                                ));
                            }
                            Some(ctx) => match ctx.create_collection(&name) {
                                Ok(_) => {
                                    let _ = request.respond(json_response(
                                        200,
                                        serde_json::json!({"ok": true}),
                                    ));
                                }
                                Err(e) => {
                                    let _ = request.respond(json_response(
                                        400,
                                        serde_json::json!({"error": format!("{}", e)}),
                                    ));
                                }
                            },
                        }
                    }
                    (Method::Post, "/api/secret/get") => {
                        let collection = str_field("collection").unwrap_or_default();
                        let name = str_field("name").unwrap_or_default();
                        if collection.is_empty() || name.is_empty() {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": "collection and name are required"}),
                            ));
                            continue;
                        }
                        let mut guard = ctx_lock.lock().unwrap();
                        match guard.as_mut() {
                            None => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": "Not authenticated"}),
                                ));
                            }
                            Some(ctx) => match ctx.api_get_secret_bytes(&collection, &name) {
                                Ok((bytes, generated)) => {
                                    let is_utf8 = std::str::from_utf8(&bytes).is_ok();
                                    let value = if is_utf8 {
                                        String::from_utf8_lossy(&bytes).trim_end().to_string()
                                    } else {
                                        "".to_string()
                                    };
                                    let b64 =
                                        base64::engine::general_purpose::STANDARD.encode(&bytes);
                                    let _ = request.respond(json_response(
                                        200,
                                        serde_json::json!({"value": value, "file_base64": b64, "is_binary": !is_utf8, "generated": generated}),
                                    ));
                                }
                                Err(e) => {
                                    let _ = request.respond(json_response(
                                        400,
                                        serde_json::json!({"error": format!("{}", e)}),
                                    ));
                                }
                            },
                        }
                    }
                    (Method::Post, "/api/secret/put") => {
                        let collection = str_field("collection").unwrap_or_default();
                        let name = str_field("name").unwrap_or_default();
                        let value_opt = str_field("value");
                        let b64_opt = str_field("file_base64");
                        if collection.is_empty() || name.is_empty() {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": "collection and name are required"}),
                            ));
                            continue;
                        }

                        let final_value = if let Some(b64) = b64_opt {
                            Some(
                                base64::engine::general_purpose::STANDARD
                                    .decode(&b64)
                                    .unwrap_or_default(),
                            )
                        } else {
                            value_opt.map(|s| s.into_bytes())
                        };

                        let mut guard = ctx_lock.lock().unwrap();
                        match guard.as_mut() {
                            None => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": "Not authenticated"}),
                                ));
                            }
                            Some(ctx) => {
                                match ctx.api_put_secret_bytes(&collection, &name, final_value) {
                                    Ok((bytes, generated)) => {
                                        let value =
                                            String::from_utf8_lossy(&bytes).trim_end().to_string();
                                        let _ = request.respond(json_response(
                                        200,
                                        serde_json::json!({"value": value, "generated": generated}),
                                    ));
                                    }
                                    Err(e) => {
                                        let _ = request.respond(json_response(
                                            400,
                                            serde_json::json!({"error": format!("{}", e)}),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    (Method::Post, "/api/batch") => {
                        let collection = str_field("collection").unwrap_or_default();
                        if collection.is_empty() {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": "collection is required"}),
                            ));
                            continue;
                        }
                        let create_if_missing = body
                            .get("create_if_missing")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let secrets: Vec<(String, Option<String>)> = body
                            .get("secrets")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .map(|item| {
                                        let name = item
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let value = item
                                            .get("value")
                                            .and_then(|v| v.as_str())
                                            .map(String::from);
                                        (name, value)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let invites: Vec<(String, Option<String>, Option<String>)> = body
                            .get("invites")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .map(|item| {
                                        let client = item
                                            .get("client")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let validity = item
                                            .get("validity")
                                            .and_then(|v| v.as_str())
                                            .filter(|s| !s.is_empty())
                                            .map(String::from);
                                        let role = item
                                            .get("role")
                                            .and_then(|v| v.as_str())
                                            .filter(|s| !s.is_empty())
                                            .map(String::from);
                                        (client, validity, role)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        let mut guard = ctx_lock.lock().unwrap();
                        match guard.as_mut() {
                            None => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": "Not authenticated"}),
                                ));
                            }
                            Some(ctx) => match ctx.api_batch(
                                &collection,
                                create_if_missing,
                                &secrets,
                                &invites,
                            ) {
                                Ok(report) => {
                                    let _ = request.respond(json_response(200, report));
                                }
                                Err(e) => {
                                    let _ = request.respond(json_response(
                                        400,
                                        serde_json::json!({"error": format!("{}", e)}),
                                    ));
                                }
                            },
                        }
                    }
                    (Method::Post, "/api/secret/delete") => {
                        let collection = str_field("collection").unwrap_or_default();
                        let name = str_field("name").unwrap_or_default();
                        if collection.is_empty() || name.is_empty() {
                            let _ = request.respond(json_response(
                                400,
                                serde_json::json!({"error": "collection and name are required"}),
                            ));
                            continue;
                        }
                        let mut guard = ctx_lock.lock().unwrap();
                        match guard.as_mut() {
                            None => {
                                let _ = request.respond(json_response(
                                    401,
                                    serde_json::json!({"error": "Not authenticated"}),
                                ));
                            }
                            Some(ctx) => match ctx.cmd_delete_secret(&collection, &name) {
                                Ok(_) => {
                                    let _ = request.respond(json_response(
                                        200,
                                        serde_json::json!({"ok": true}),
                                    ));
                                }
                                Err(e) => {
                                    let _ = request.respond(json_response(
                                        400,
                                        serde_json::json!({"error": format!("{}", e)}),
                                    ));
                                }
                            },
                        }
                    }
                    _ => {
                        let _ = request.respond(json_response(
                            404,
                            serde_json::json!({"error": "Not found"}),
                        ));
                    }
                }
                continue;
            }

            let path = if path_only == "/" {
                "index.html"
            } else {
                path_only.trim_start_matches('/')
            };

            match Asset::get(path) {
                Some(content) => {
                    let mime = mime_guess::from_path(path).first_or_octet_stream();
                    let response = Response::from_data(content.data.into_owned()).with_header(
                        Header::from_bytes(&b"Content-Type"[..], mime.as_ref().as_bytes()).unwrap(),
                    );
                    let _ = request.respond(response);
                }
                None => {
                    if let Some(content) = Asset::get("index.html") {
                        let mime = mime_guess::from_path("index.html").first_or_octet_stream();
                        let response = Response::from_data(content.data.into_owned()).with_header(
                            Header::from_bytes(&b"Content-Type"[..], mime.as_ref().as_bytes())
                                .unwrap(),
                        );
                        let _ = request.respond(response);
                    } else {
                        let response = Response::from_string("Not Found").with_status_code(404);
                        let _ = request.respond(response);
                    }
                }
            }
        }
        Ok(())
    }

    fn cmd_reset_client(&mut self, target_client: &str) -> Result<String, Box<dyn Error>> {
        let mut reset_secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut reset_secret);
        let token_b64 = B64URL.encode(&reset_secret);

        let resp = self
            .client
            .post(format!(
                "{}/vault/v1/client/{}/reset-token",
                self.base_url,
                url_encode(target_client)
            ))
            .bearer_auth(&self.session.as_ref().unwrap().token)
            .json(&serde_json::json!({"token": token_b64}))
            .send()?;

        if !resp.status().is_success() {
            return Err(format!("Failed to generate reset token: {}", resp.text()?).into());
        }
        Ok(token_b64)
    }

    fn run_command(&mut self, command: Commands) -> Result<(), Box<dyn Error>> {
        match &command {
            Commands::Ui => {
                self.cmd_ui()?;
                return Ok(());
            }
            Commands::Update(args) => {
                update::handle_update(args.clone(), "rescile-vault")?;
                return Ok(());
            }
            _ => self.authenticate()?,
        }
        match command {
            Commands::Secret { action } => match action {
                SecretCommands::List { collection } => self.cmd_list_secrets(&collection)?,
                SecretCommands::Get {
                    collection,
                    secret_name,
                    file,
                } => self.cmd_get(&collection, &secret_name, file.as_ref())?,
                SecretCommands::Put {
                    collection,
                    secret_name,
                    secret_value,
                    file,
                } => self.cmd_put(&collection, &secret_name, secret_value, file.as_ref())?,
                SecretCommands::Delete {
                    collection,
                    secret_name,
                } => self.cmd_delete_secret(&collection, &secret_name)?,
            },
            Commands::Collection { action } => match action {
                CollectionCommands::List => self.cmd_list_collections()?,
                CollectionCommands::Create { name } => self.create_collection(&name)?,
                CollectionCommands::Invite {
                    name,
                    client,
                    validity,
                    role,
                } => self.invite_client(&name, &client, validity, role)?,
                CollectionCommands::Delete { name } => self.cmd_delete_collection(&name)?,
                CollectionCommands::RemoveClient { name, client } => {
                    self.cmd_remove_client(&name, &client)?
                }
                CollectionCommands::Role { name, client, role } => {
                    self.cmd_collection_role(&name, &client, &role)?
                }
                CollectionCommands::Revoke { name, client } => {
                    self.cmd_collection_revoke(&name, &client)?
                }
            },
            Commands::Client { action } => match action {
                ClientCommands::List { collection } => self.cmd_list_clients(&collection)?,
                ClientCommands::Delete { client } => self.cmd_delete_client(client.as_deref())?,
                ClientCommands::Reset { client } => {
                    let token = self.cmd_reset_client(&client)?;
                    println!("{}", token);
                }
            },
            Commands::Batch { file } => {
                let content = std::fs::read_to_string(file)?;
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let args = split_args(line);
                    let mut full_args = vec!["rescile-vault".to_string()];
                    full_args.extend(args);

                    let batch_cli = match Cli::try_parse_from(full_args) {
                        Ok(cli) => cli,
                        Err(e) => {
                            eprintln!("Failed to parse batch command: {}", e);
                            continue;
                        }
                    };
                    self.run_command(batch_cli.command)?;
                }
            }
            Commands::Ui | Commands::Update(_) => {
                // Handled above
            }
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let mut ctx = VaultContext::new(cli.clone());
    ctx.run_command(cli.command)?;
    Ok(())
}
