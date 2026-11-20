//! User-scoped DPAPI credentials; the renderer receives opaque references only.
use crate::models::ApiKeys;
use base64::{engine::general_purpose::STANDARD, Engine};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::RandomState,
    fs,
    hash::{BuildHasher, Hash, Hasher},
    path::PathBuf,
    sync::OnceLock,
};
static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());
static REFERENCES: OnceLock<RandomState> = OnceLock::new();
#[derive(Serialize, Deserialize)]
struct Envelope {
    protection: String,
    ciphertext: String,
}
pub fn init(file: PathBuf) {
    let _ = PATH.set(file);
}

#[cfg(windows)]
fn crypt(bytes: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| "Credenciais excedem o limite")?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        let result = if protect {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        result.map_err(|_| {
            "Não foi possível acessar as credenciais protegidas desta conta Windows".to_string()
        })?;
        let value = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        for index in 0..output.cbData as usize {
            std::ptr::write_volatile(output.pbData.add(index), 0);
        }
        let _ = LocalFree(HLOCAL(output.pbData.cast()));
        Ok(value)
    }
}
#[cfg(not(windows))]
fn crypt(_: &[u8], _: bool) -> Result<Vec<u8>, String> {
    Err("Armazenamento seguro requer Windows nesta versão".into())
}

fn encoded(keys: &ApiKeys) -> Result<Vec<u8>, String> {
    let mut plain = serde_json::to_vec(keys).map_err(|_| "Credenciais inválidas")?;
    let encrypted = crypt(&plain, true);
    plain.fill(0);
    serde_json::to_vec(&Envelope {
        protection: "dpapi-current-user-v1".into(),
        ciphertext: STANDARD.encode(encrypted?),
    })
    .map_err(|_| "Falha ao proteger credenciais".into())
}
fn decoded(bytes: &[u8]) -> Result<ApiKeys, String> {
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| "Credenciais protegidas inválidas")?;
    if envelope.protection != "dpapi-current-user-v1" {
        return Err("Formato de proteção desconhecido".into());
    }
    let mut plain = crypt(
        &STANDARD
            .decode(envelope.ciphertext)
            .map_err(|_| "Credenciais inválidas")?,
        false,
    )?;
    let result = serde_json::from_slice::<ApiKeys>(&plain)
        .map(ApiKeys::normalized)
        .map_err(|_| "Credenciais protegidas inválidas".into());
    plain.fill(0);
    result
}
pub fn load() -> Result<ApiKeys, String> {
    let _guard = LOCK.lock();
    let file = PATH.get().ok_or("Diretório de credenciais indisponível")?;
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ApiKeys::default()),
        Err(_) => return Err("Não foi possível ler as credenciais".into()),
    };
    if serde_json::from_slice::<Envelope>(&bytes).is_ok() {
        return decoded(&bytes);
    }
    let keys = serde_json::from_slice::<ApiKeys>(&bytes)
        .map_err(|_| "Arquivo de credenciais inválido; restaure um backup")?
        .normalized();
    // Verify an encrypted recovery copy before replacing legacy plaintext.
    let encrypted = encoded(&keys)?;
    let recovery = file.with_extension("migration.dpapi");
    crate::storage::atomic_write(&recovery, &encrypted)?;
    let verified = decoded(&fs::read(&recovery).map_err(|_| "Falha ao verificar backup")?)?;
    if serde_json::to_value(&verified).ok() != serde_json::to_value(&keys).ok() {
        return Err("Backup de credenciais divergente".into());
    }
    let replacement = file.with_extension("encrypted.tmp");
    crate::storage::atomic_write(&replacement, &encrypted)?;
    fs::rename(replacement, file)
        .map_err(|_| "Não foi possível concluir a migração das credenciais")?;
    Ok(keys)
}
pub fn save(keys: &ApiKeys) -> Result<(), String> {
    let _guard = LOCK.lock();
    let file = PATH.get().ok_or("Diretório de credenciais indisponível")?;
    if file.exists() {
        decoded(&fs::read(file).map_err(|_| "Não foi possível ler as credenciais atuais")?)?;
    }
    crate::storage::atomic_write(file, &encoded(keys)?)
}
fn reference(provider: &str, key: &str) -> String {
    let mut hash = REFERENCES.get_or_init(RandomState::new).build_hasher();
    provider.hash(&mut hash);
    key.hash(&mut hash);
    format!("stored:{:016x}", hash.finish())
}
pub fn mask(keys: &ApiKeys) -> ApiKeys {
    let map = |provider, values: &Vec<String>| {
        values
            .iter()
            .map(|value| reference(provider, value))
            .collect()
    };
    ApiKeys {
        groq: map("groq", &keys.groq),
        deepgram: map("deepgram", &keys.deepgram),
        openrouter: map("openrouter", &keys.openrouter),
        assemblyai: map("assemblyai", &keys.assemblyai),
        elevenlabs: map("elevenlabs", &keys.elevenlabs),
        meta: map("meta", &keys.meta),
    }
}
pub fn resolve(
    provider: &str,
    values: Vec<String>,
    existing: &[String],
) -> Result<Vec<String>, String> {
    values
        .into_iter()
        .map(|value| {
            if value.starts_with("stored:") {
                existing
                    .iter()
                    .find(|key| reference(provider, key) == value)
                    .cloned()
                    .ok_or_else(|| {
                        "As credenciais mudaram. Recarregue as configurações antes de salvar."
                            .into()
                    })
            } else {
                Ok(value)
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_reject_stale_and_cross_provider_values() {
        let keys = vec!["synthetic-test-credential".to_string()];
        let token = reference("assemblyai", &keys[0]);
        assert!(!token.contains(&keys[0]));
        assert_eq!(
            resolve("assemblyai", vec![token.clone()], &keys).unwrap(),
            keys
        );
        assert!(resolve("openrouter", vec![token.clone()], &keys).is_err());
        assert!(resolve("assemblyai", vec![token], &[]).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip_and_tampering() {
        let bytes = b"synthetic-test-only";
        let encrypted = crypt(bytes, true).unwrap();
        assert_ne!(encrypted, bytes);
        assert_eq!(crypt(&encrypted, false).unwrap(), bytes);
        let mut altered = encrypted;
        altered[0] ^= 1;
        assert!(crypt(&altered, false).is_err());
    }
}
