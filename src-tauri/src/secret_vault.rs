use keyring::{Entry, Error as KeyringError};
use zeroize::Zeroizing;

use crate::app_error::AppError;

// Keep this service identifier stable so previously saved provider credentials remain readable.
const KEYRING_SERVICE: &str = "com.semantix.companion.provider-api-key";

pub(crate) struct SecretVault;

impl SecretVault {
    pub(crate) fn store(secret_ref: &str, secret: &str) -> Result<(), AppError> {
        Self::entry(secret_ref)?
            .set_password(secret)
            .map_err(AppError::keyring)
    }

    pub(crate) fn delete(secret_ref: &str) -> Result<(), AppError> {
        match Self::entry(secret_ref)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(AppError::keyring(error)),
        }
    }

    pub(crate) fn get(secret_ref: &str) -> Result<Zeroizing<String>, AppError> {
        Self::entry(secret_ref)?
            .get_password()
            .map(Zeroizing::new)
            .map_err(AppError::keyring)
    }

    fn entry(secret_ref: &str) -> Result<Entry, AppError> {
        Entry::new(KEYRING_SERVICE, secret_ref).map_err(AppError::keyring)
    }
}
