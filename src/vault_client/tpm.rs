use std::convert::TryFrom;
use std::fs;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::str::FromStr;

use tss_esapi::{
    attributes::object::ObjectAttributesBuilder,
    constants::{
        capabilities::CapabilityType, response_code::Tss2ResponseCodeKind,
        startup_type::StartupType,
    },
    handles::{ObjectHandle, PersistentTpmHandle, TpmHandle},
    interface_types::{
        algorithm::{HashingAlgorithm, PublicAlgorithm},
        key_bits::RsaKeyBits,
        resource_handles::{Hierarchy, Provision},
        session_handles::AuthSession,
    },
    structures::{
        Auth, CapabilityData, Data, HashScheme, Public, PublicBuilder, PublicKeyRsa,
        PublicRsaParameters, PublicRsaParametersBuilder, RsaDecryptionScheme, RsaExponent,
        RsaScheme, SymmetricDefinitionObject,
    },
    tcti_ldr::{DeviceConfig, TctiNameConf},
    Context,
};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;

const RSA_KEY_AUTH: &[u8] = b"rescile-vault-tpm-v1";
const PERSISTENT_HANDLE_START: u32 = 0x81008000;
const PERSISTENT_HANDLE_END: u32 = 0x8100FFFF;

fn tpm_state_dir() -> PathBuf {
    let dir = dirs_candidate();
    fs::create_dir_all(&dir).ok();
    dir
}

fn dirs_candidate() -> PathBuf {
    if let Ok(d) = std::env::var("RESCILE_VAULT_TPM_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".rescile-vault").join("tpm")
}

fn wrapped_key_path(client_name: &str) -> PathBuf {
    tpm_state_dir().join(format!("{}.tpm-wrapped-key", client_name))
}

fn build_public(
    attrs: tss_esapi::attributes::object::ObjectAttributes,
    params: PublicRsaParameters,
) -> tss_esapi::Result<Public> {
    PublicBuilder::new()
        .with_public_algorithm(PublicAlgorithm::Rsa)
        .with_object_attributes(attrs)
        .with_name_hashing_algorithm(HashingAlgorithm::Sha256)
        .with_rsa_parameters(params)
        .with_rsa_unique_identifier(PublicKeyRsa::default())
        .build()
}

struct TpmKeyWrapper {
    ctx: Context,
}

impl Drop for TpmKeyWrapper {
    fn drop(&mut self) {
        // swtpm (vTPM) may need explicit shutdown; hardware TPM usually does it automatically.
        // We attempt shutdown and ignore errors for hardware TPM.
        let _ = self.ctx.shutdown(StartupType::Clear);
    }
}

fn device_is_accessible(path: &str) -> bool {
    OpenOptions::new().read(true).write(true).open(path).is_ok()
}

impl TpmKeyWrapper {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Try hardware TPM first (checking actual access, not just existence),
        // then fall back to swtpm
        let (tcti, is_vtpm_device) = if device_is_accessible("/dev/tpmrm0") {
            (
                TctiNameConf::Device(DeviceConfig::from_str("/dev/tpmrm0")?),
                false,
            )
        } else if std::path::Path::new("/dev/tpmrm0").exists() {
            // Device exists but is not accessible — try swtpm before giving up
            eprintln!(
                "Warning: /dev/tpmrm0 exists but is not accessible (permission denied). \
                 Consider adding your user to the 'tss' group or running with appropriate permissions. \
                 Falling back to swtpm."
            );
            (
                TctiNameConf::Swtpm(tss_esapi::tcti_ldr::NetworkTPMConfig::default()),
                true,
            )
        } else if device_is_accessible("/dev/tpm0") {
            (TctiNameConf::Device(DeviceConfig::default()), false)
        } else if std::path::Path::new("/dev/tpm0").exists() {
            eprintln!(
                "Warning: /dev/tpm0 exists but is not accessible (permission denied). \
                 Falling back to swtpm."
            );
            (
                TctiNameConf::Swtpm(tss_esapi::tcti_ldr::NetworkTPMConfig::default()),
                true,
            )
        } else {
            // Try swtpm on default port
            (
                TctiNameConf::Swtpm(tss_esapi::tcti_ldr::NetworkTPMConfig::default()),
                true,
            )
        };

        let mut ctx = Context::new(tcti)?;

        // For vTPM we need to call startup; for hardware TPM the platform usually does it
        if is_vtpm_device {
            // Ignore error if already started
            let _ = ctx.startup(StartupType::Clear);
        }

        Ok(Self { ctx })
    }

    fn ensure_rsa_key(&mut self) -> tss_esapi::Result<ObjectHandle> {
        self.ctx.clear_sessions();

        let mut value = PERSISTENT_HANDLE_START;
        let mut srk: Option<ObjectHandle> = None;

        loop {
            let (data, more) = self.ctx.get_capability(CapabilityType::Handles, value, 8)?;

            if let CapabilityData::Handles(h) = data {
                let handles = h.into_inner();
                if handles.is_empty() && srk.is_none() {
                    let new_srk = self.create_srk()?;
                    return self.create_rsa_key(new_srk);
                }

                if let Some(last) = handles.last() {
                    value = u32::from(*last) + 1;
                }

                for handle in handles {
                    if let TpmHandle::Persistent(_) = handle {
                        let target_handle = self.ctx.tr_from_tpm_public(handle)?;
                        let (public, _, _) = self.ctx.read_public(target_handle.into())?;

                        if let Public::Rsa {
                            object_attributes, ..
                        } = public
                        {
                            let a = object_attributes;
                            // Match RSA wrapping key attributes
                            if !a.restricted()
                                && a.fixed_tpm()
                                && a.fixed_parent()
                                && a.sensitive_data_origin()
                                && a.user_with_auth()
                                && a.decrypt()
                            {
                                return Ok(target_handle);
                            }
                            // Match SRK attributes
                            if a.restricted()
                                && a.decrypt()
                                && a.fixed_tpm()
                                && a.fixed_parent()
                                && a.sensitive_data_origin()
                                && a.no_da()
                            {
                                srk = Some(target_handle);
                            }
                        }
                    }
                }
            }

            if !more {
                let parent = match srk {
                    Some(h) => h,
                    None => self.create_srk()?,
                };
                return self.create_rsa_key(parent);
            }
        }
    }

    fn make_persistent(
        &mut self,
        handle: impl Into<ObjectHandle>,
    ) -> tss_esapi::Result<ObjectHandle> {
        let mut value = PERSISTENT_HANDLE_START;
        let handle = handle.into();

        loop {
            if value > PERSISTENT_HANDLE_END {
                return Err(tss_esapi::Error::WrapperError(
                    tss_esapi::WrapperErrorKind::InternalError,
                ));
            }
            match self
                .ctx
                .execute_with_session(Some(AuthSession::Password), |ctx| {
                    ctx.evict_control(
                        Provision::Owner,
                        handle,
                        PersistentTpmHandle::new(value)?.into(),
                    )
                }) {
                Ok(h) => {
                    let _ = self.ctx.flush_context(handle);
                    return Ok(h);
                }
                Err(e) => {
                    if matches!(
                        &e,
                        tss_esapi::Error::Tss2Error(code)
                            if code.kind() == Some(Tss2ResponseCodeKind::NvDefined)
                    ) {
                        value += 1;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }

    fn create_rsa_key(&mut self, hsrk: ObjectHandle) -> tss_esapi::Result<ObjectHandle> {
        let attrs = ObjectAttributesBuilder::new()
            .with_fixed_tpm(true)
            .with_fixed_parent(true)
            .with_user_with_auth(true)
            .with_sensitive_data_origin(true)
            .with_decrypt(true)
            .build()?;

        let hash_algo = HashScheme::new(HashingAlgorithm::Sha256);
        let params = PublicRsaParametersBuilder::new()
            .with_scheme(RsaScheme::Oaep(hash_algo))
            .with_key_bits(RsaKeyBits::Rsa2048)
            .with_exponent(RsaExponent::ZERO_EXPONENT)
            .build()?;

        let key_result = self
            .ctx
            .execute_with_session(Some(AuthSession::Password), |ctx| {
                let result = ctx.create(
                    hsrk.into(),
                    build_public(attrs, params)?,
                    Some(Auth::try_from(RSA_KEY_AUTH)?),
                    None,
                    None,
                    None,
                )?;
                ctx.load(hsrk.into(), result.out_private, result.out_public)
            })?;

        self.make_persistent(key_result)
    }

    fn create_srk(&mut self) -> tss_esapi::Result<ObjectHandle> {
        let attrs = ObjectAttributesBuilder::new()
            .with_fixed_tpm(true)
            .with_fixed_parent(true)
            .with_no_da(true)
            .with_restricted(true)
            .with_user_with_auth(true)
            .with_sensitive_data_origin(true)
            .with_decrypt(true)
            .build()?;

        let params = PublicRsaParametersBuilder::new_restricted_decryption_key(
            SymmetricDefinitionObject::AES_128_CFB,
            RsaKeyBits::Rsa2048,
            RsaExponent::ZERO_EXPONENT,
        )
        .build()?;

        let key_result = self
            .ctx
            .execute_with_session(Some(AuthSession::Password), |ctx| {
                ctx.create_primary(
                    Hierarchy::Owner,
                    build_public(attrs, params)?,
                    None,
                    None,
                    None,
                    None,
                )
            })?;

        self.make_persistent(key_result.key_handle)
    }

    fn wrap_key(&mut self, target_key: [u8; 32]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let hrsa = self.ensure_rsa_key()?;
        let wrapped = self.ctx.execute_with_nullauth_session(|ctx| {
            ctx.rsa_encrypt(
                hrsa.into(),
                PublicKeyRsa::try_from(target_key.as_slice())?,
                RsaDecryptionScheme::Null,
                Data::default(),
            )
        })?;
        Ok(wrapped.to_vec())
    }

    fn unwrap_key(&mut self, wrapped: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let hrsa = self.ensure_rsa_key()?;
        self.ctx.tr_set_auth(hrsa, Auth::try_from(RSA_KEY_AUTH)?)?;
        let unwrapped = self.ctx.execute_with_nullauth_session(|ctx| {
            ctx.rsa_decrypt(
                hrsa.into(),
                PublicKeyRsa::try_from(wrapped)?,
                RsaDecryptionScheme::Null,
                Data::default(),
            )
        })?;
        Ok(unwrapped.to_vec())
    }
}

/// Derive a 32-byte master key using TPM.
/// On first use, generates a random master key and wraps it with TPM.
/// On subsequent uses, unwraps the stored key.
/// Returns (auth_key, key_encryption_key) just like derive_keys().
pub fn tpm_derive_keys(
    client_name: &str,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    let path = wrapped_key_path(client_name);
    let mut wrapper = TpmKeyWrapper::new()?;

    let master_key: [u8; 32] = if path.exists() {
        let stored = fs::read_to_string(&path)?;
        let wrapped_bytes = B64URL.decode(stored.trim())?;
        let unwrapped = wrapper.unwrap_key(&wrapped_bytes)?;
        <[u8; 32]>::try_from(unwrapped.as_slice())
            .map_err(|_| "TPM unwrapped key has wrong length")?
    } else {
        let mut mk = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mk);
        let wrapped = wrapper.wrap_key(mk)?;
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, B64URL.encode(&wrapped))?;
        mk
    };

    // Derive auth key and KEK from master key, same as password-based flow
    let hkdf_auth = hkdf::Hkdf::<sha2::Sha256>::new(None, &master_key);
    let mut ak = [0u8; 32];
    hkdf_auth
        .expand(b"rescile-vault-auth-v1", &mut ak)
        .map_err(|e| format!("HKDF expand error: {}", e))?;

    let hkdf_enc = hkdf::Hkdf::<sha2::Sha256>::new(None, &master_key);
    let mut kek = [0u8; 32];
    hkdf_enc
        .expand(b"rescile-vault-enc-v1", &mut kek)
        .map_err(|e| format!("HKDF expand error: {}", e))?;

    Ok((ak.to_vec(), kek.to_vec()))
}
