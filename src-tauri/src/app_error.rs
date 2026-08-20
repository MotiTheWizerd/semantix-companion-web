use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub(crate) struct AppError {
    message: String,
}

impl AppError {
    pub(crate) fn validation(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn database(error: rusqlite::Error) -> Self {
        Self {
            message: format!("Could not update Companion's local data: {error}"),
        }
    }

    pub(crate) fn keyring(error: keyring::Error) -> Self {
        Self {
            message: format!("Could not access the operating system keychain: {error}"),
        }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            message: format!(
                "Companion could not complete the operation: {}",
                message.into()
            ),
        }
    }
}

impl Display for AppError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.to_string()
    }
}
