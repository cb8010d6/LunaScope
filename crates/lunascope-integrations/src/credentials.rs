use std::fmt;

use lunascope_core::CredentialReference;
use thiserror::Error;
use zeroize::Zeroize;

const SERVICE_PREFIX: &str = "io.lunascope.provider";

pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CredentialError::EmptySecret);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, Default)]
pub struct KeyringCredentialStore;

impl KeyringCredentialStore {
    pub fn put(
        &self,
        reference: &CredentialReference,
        secret: &SecretValue,
    ) -> Result<(), CredentialError> {
        entry(reference)?.set_password(secret.expose())?;
        Ok(())
    }

    pub fn get(&self, reference: &CredentialReference) -> Result<SecretValue, CredentialError> {
        match entry(reference)?.get_password() {
            Ok(secret) => SecretValue::new(secret),
            Err(keyring::Error::NoEntry) if can_use_legacy_entry(reference) => {
                let secret = legacy_entry(reference)?.get_password()?;
                entry(reference)?.set_password(&secret)?;
                SecretValue::new(secret)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn get_optional(
        &self,
        reference: &CredentialReference,
    ) -> Result<Option<SecretValue>, CredentialError> {
        match entry(reference)?.get_password() {
            Ok(secret) => SecretValue::new(secret).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        entry(reference)?.delete_credential()?;
        Ok(())
    }
}

fn entry(reference: &CredentialReference) -> Result<keyring::Entry, CredentialError> {
    validate_reference(reference)?;
    Ok(keyring::Entry::new(
        &format!("{SERVICE_PREFIX}.{}", provider_key(reference)),
        &reference.id,
    )?)
}

fn provider_key(reference: &CredentialReference) -> &'static str {
    use lunascope_core::ProviderType;
    match reference.provider {
        ProviderType::OpenAi => "openai",
        ProviderType::Anthropic => "anthropic",
        ProviderType::DeepSeek => "deepseek",
        ProviderType::GenericOpenAiCompatible => "generic-openai",
        ProviderType::GenericAnthropicCompatible => "generic-anthropic",
    }
}

fn can_use_legacy_entry(reference: &CredentialReference) -> bool {
    let provider = provider_key(reference);
    reference.id == provider || reference.id == format!("{provider}-primary")
}

fn legacy_entry(reference: &CredentialReference) -> Result<keyring::Entry, CredentialError> {
    Ok(keyring::Entry::new("lunascope", provider_key(reference))?)
}

fn validate_reference(reference: &CredentialReference) -> Result<(), CredentialError> {
    let valid = !reference.id.is_empty()
        && reference.id.len() <= 128
        && reference
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(CredentialError::InvalidReference(reference.id.clone()))
    }
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("secret cannot be empty")]
    EmptySecret,
    #[error("credential reference is invalid: {0}")]
    InvalidReference(String),
    #[error(transparent)]
    Keyring(#[from] keyring::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::ProviderType;

    #[test]
    fn secret_debug_is_always_redacted() {
        let secret = SecretValue::new("known-canary").expect("secret");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        assert_eq!(secret.expose(), "known-canary");
    }

    #[test]
    fn legacy_fallback_is_limited_to_default_provider_references() {
        let default = CredentialReference {
            id: "deepseek-primary".into(),
            provider: ProviderType::DeepSeek,
            label: String::new(),
        };
        let custom = CredentialReference {
            id: "research-lab".into(),
            provider: ProviderType::DeepSeek,
            label: String::new(),
        };
        assert!(can_use_legacy_entry(&default));
        assert!(!can_use_legacy_entry(&custom));
    }

    #[test]
    fn reference_validation_rejects_keyring_namespace_injection() {
        let reference = CredentialReference {
            id: "bad/reference".into(),
            provider: ProviderType::OpenAi,
            label: "Bad".into(),
        };
        assert!(matches!(
            validate_reference(&reference),
            Err(CredentialError::InvalidReference(_))
        ));
    }

    #[test]
    #[ignore = "writes and deletes a random canary in the Windows Credential Manager"]
    fn windows_keyring_round_trip() {
        let reference = CredentialReference {
            id: format!("test-{}", uuid::Uuid::new_v4()),
            provider: ProviderType::OpenAi,
            label: "LunaScope integration test".into(),
        };
        let store = KeyringCredentialStore;
        let secret = SecretValue::new(format!("canary-{}", uuid::Uuid::new_v4())).expect("secret");
        store.put(&reference, &secret).expect("put");
        let recovered = store.get(&reference).expect("get");
        assert_eq!(recovered.expose(), secret.expose());
        store.delete(&reference).expect("delete");
        assert!(
            store
                .get_optional(&reference)
                .expect("optional get")
                .is_none()
        );
    }
}
