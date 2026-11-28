use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::{Validate, ValidationError, ValidationErrors};
use webauthn_rs::prelude::{PasskeyAuthentication, PublicKeyCredential};

use super::{
    passkey::{self, Ceremony},
    password,
};
use crate::{db, error::AppError, state::AppState};

const PASSWORD_MAX: usize = 128;

/// The password, or one of the user's passkeys in its place, asked again before a change that
/// would let someone holding only a stolen session cookie keep access or lock the owner out, such
/// as adding a passkey. Send one of the two.
#[derive(Deserialize, ToSchema)]
pub struct Reauth {
    pub password: Option<String>,
    /// `toJSON()` of the credential `navigator.credentials.get()` returned for the options from
    /// `/api/auth/passkeys/reauth/start`, sent from the same browser
    #[schema(value_type = Option<std::collections::HashMap<String, serde_json::Value>>)]
    pub passkey: Option<PublicKeyCredential>,
}

impl Reauth {
    pub const fn is_empty(&self) -> bool {
        self.password.is_none() && self.passkey.is_none()
    }

    /// Adds what's wrong with it to `errors`, for a body that holds it beside other fields
    pub fn check(&self, errors: &mut ValidationErrors) {
        let problem = match (&self.password, &self.passkey) {
            (Some(password), None) if !(1..=PASSWORD_MAX).contains(&password.chars().count()) => {
                Some(("password", "must be between 1 and 128 characters"))
            }
            (None, None) => Some(("password", "send the password or a passkey")),
            (Some(_), Some(_)) => Some(("passkey", "send the password or a passkey, not both")),
            _ => None,
        };
        if let Some((field, message)) = problem {
            errors.add(
                field,
                ValidationError::new(field).with_message(message.into()),
            );
        }
    }
}

impl Validate for Reauth {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        self.check(&mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Checks the password or passkey in `reauth`. A passkey's challenge is spent either way.
pub async fn confirm(
    state: &AppState,
    jar: &CookieJar,
    user_id: Uuid,
    reauth: Reauth,
) -> Result<(), AppError> {
    reauth.validate()?;
    // Otherwise someone holding a stolen session cookie could guess at the password unhindered
    state.rate_limits.reauth(user_id)?;
    match reauth.passkey {
        Some(credential) => confirm_passkey(state, jar, user_id, &credential).await,
        None => confirm_password(state, user_id, reauth.password.unwrap_or_default()).await,
    }
}

async fn confirm_password(
    state: &AppState,
    user_id: Uuid,
    password: String,
) -> Result<(), AppError> {
    let hash = db::users::password_hash(&state.pool, user_id).await?;
    if password::verify(password, Some(hash)).await? {
        Ok(())
    } else {
        Err(AppError::IncorrectPassword)
    }
}

async fn confirm_passkey(
    state: &AppState,
    jar: &CookieJar,
    user_id: Uuid,
    credential: &PublicKeyCredential,
) -> Result<(), AppError> {
    let authentication: PasskeyAuthentication = passkey::finish(
        &state.pool,
        jar,
        Ceremony::Reauthentication,
        Some(user_id),
        state.secure_cookies(),
    )
    .await?
    .ok_or(AppError::IncorrectPasskey)?;
    let result = passkey::relying_party(&state.app_url)?
        .finish_passkey_authentication(credential, &authentication)
        .map_err(|error| {
            tracing::info!(?error, "passkey re-check failed verification");
            AppError::IncorrectPasskey
        })?;

    let mut tx = state.pool.begin().await?;
    // The challenge named the passkeys the user had when it began, and one may be gone since
    let (passkey_id, mut stored, _) =
        db::passkeys::find_for_update(&mut *tx, result.cred_id().as_ref())
            .await?
            .filter(|(_, _, owner)| owner.id == user_id)
            .ok_or(AppError::IncorrectPasskey)?;
    stored.update_credential(&result);
    db::passkeys::record_use(&mut *tx, passkey_id, &stored).await?;
    tx.commit().await?;

    Ok(())
}
