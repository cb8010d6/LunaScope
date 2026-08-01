use std::fmt;

use thiserror::Error;
use zeroize::Zeroize;

const SERVICE_PREFIX: &str = "io.lunascope.mcp";

pub struct McpSecretValue(String);

impl McpSecretValue {
    pub fn new(value: impl Into<String>) -> Result<Self, McpCredentialError> {
        let value = value.into();
        if value.is_empty() {
            return Err(McpCredentialError::EmptySecret);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for McpSecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Drop for McpSecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, Default)]
pub struct McpCredentialStore;

impl McpCredentialStore {
    pub fn put(
        &self,
        server_id: &str,
        reference_id: &str,
        secret: &McpSecretValue,
    ) -> Result<(), McpCredentialError> {
        entry(server_id, reference_id)?.set_password(secret.expose())?;
        Ok(())
    }

    pub fn get(
        &self,
        server_id: &str,
        reference_id: &str,
    ) -> Result<McpSecretValue, McpCredentialError> {
        McpSecretValue::new(entry(server_id, reference_id)?.get_password()?)
    }

    pub fn get_optional(
        &self,
        server_id: &str,
        reference_id: &str,
    ) -> Result<Option<McpSecretValue>, McpCredentialError> {
        match entry(server_id, reference_id)?.get_password() {
            Ok(secret) => McpSecretValue::new(secret).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn delete(&self, server_id: &str, reference_id: &str) -> Result<(), McpCredentialError> {
        entry(server_id, reference_id)?.delete_credential()?;
        Ok(())
    }
}

fn entry(server_id: &str, reference_id: &str) -> Result<keyring::Entry, McpCredentialError> {
    validate_component("server", server_id)?;
    validate_component("credential reference", reference_id)?;
    Ok(keyring::Entry::new(
        &format!("{SERVICE_PREFIX}.{server_id}"),
        reference_id,
    )?)
}

fn validate_component(label: &'static str, value: &str) -> Result<(), McpCredentialError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(McpCredentialError::InvalidComponent {
            label,
            value: value.into(),
        })
    }
}

#[derive(Debug, Error)]
pub enum McpCredentialError {
    #[error("secret cannot be empty")]
    EmptySecret,
    #[error("{label} is invalid: {value}")]
    InvalidComponent { label: &'static str, value: String },
    #[error(transparent)]
    Keyring(#[from] keyring::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted_and_namespace_injection_is_rejected() {
        let secret = McpSecretValue::new("mcp-canary").expect("secret");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        assert!(entry("bad/server", "key").is_err());
        assert!(entry("server", "bad/key").is_err());
    }

    #[test]
    #[ignore = "writes and deletes a random canary in Windows Credential Manager"]
    fn windows_mcp_keyring_round_trip() {
        let server_id = format!("test-{}", uuid::Uuid::new_v4());
        let reference_id = "authorization";
        let store = McpCredentialStore;
        let secret =
            McpSecretValue::new(format!("canary-{}", uuid::Uuid::new_v4())).expect("secret");
        store
            .put(&server_id, reference_id, &secret)
            .expect("put credential");
        assert_eq!(
            store
                .get(&server_id, reference_id)
                .expect("get credential")
                .expose(),
            secret.expose()
        );
        store
            .delete(&server_id, reference_id)
            .expect("delete credential");
        assert!(
            store
                .get_optional(&server_id, reference_id)
                .expect("optional get")
                .is_none()
        );
    }
}
