use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, types::Json};
use uuid::Uuid;

use crate::agents::Grant;

pub struct NewSignIn<'a> {
    pub code_hash: &'a [u8],
    pub user_id: Uuid,
    pub client_id: &'a str,
    pub client_name: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub target: Target<'a>,
    pub expires_at: DateTime<Utc>,
}

/// What the owner agreed to on the consent page
#[derive(Clone, Copy)]
pub enum Target<'a> {
    /// Sign in again to a connection the same client already has
    Existing(Uuid),
    /// A new connection, created only when the code is exchanged
    New { name: &'a str, grant: &'a Grant },
}

/// An authorization code's row, as read under its lock
pub struct SignIn {
    pub id: Uuid,
    pub user_id: Uuid,
    pub client_id: String,
    pub client_name: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub connection_id: Option<Uuid>,
    pub name: Option<String>,
    pub grant: Option<serde_json::Value>,
    pub used: bool,
    pub expired: bool,
}

pub async fn create(db: impl PgExecutor<'_>, new: NewSignIn<'_>) -> sqlx::Result<()> {
    let (connection_id, name, grant) = match new.target {
        Target::Existing(id) => (Some(id), None, None),
        Target::New { name, grant } => (None, Some(name), Some(Json(grant))),
    };
    sqlx::query!(
        r#"INSERT INTO agent_sign_ins
               (code_hash, user_id, client_id, client_name, redirect_uri, code_challenge,
                connection_id, name, "grant", expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        new.code_hash,
        new.user_id,
        new.client_id,
        new.client_name,
        new.redirect_uri,
        new.code_challenge,
        connection_id,
        name,
        grant as _,
        new.expires_at,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Locks the code's row until the transaction ends, so two exchanges of one code can't both win
pub async fn find_locked(
    transaction: &mut PgConnection,
    code_hash: &[u8],
) -> sqlx::Result<Option<SignIn>> {
    sqlx::query_as!(
        SignIn,
        r#"SELECT id, user_id, client_id, client_name, redirect_uri, code_challenge,
               connection_id, name, "grant", used_at IS NOT NULL AS "used!",
               expires_at <= now() AS "expired!"
           FROM agent_sign_ins WHERE code_hash = $1 FOR UPDATE"#,
        code_hash
    )
    .fetch_optional(transaction)
    .await
}

/// Notes the connection the code led to, for [`find_locked`] to hand back should it be used again
pub async fn mark_used(db: impl PgExecutor<'_>, id: Uuid, connection_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE agent_sign_ins SET used_at = now(), connection_id = $2 WHERE id = $1",
        id,
        connection_id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Codes stay a day past their minute, so a late second use is still caught
pub async fn delete_expired(db: impl PgExecutor<'_>) -> sqlx::Result<u64> {
    let deleted =
        sqlx::query!("DELETE FROM agent_sign_ins WHERE expires_at < now() - interval '1 day'")
            .execute(db)
            .await?;
    Ok(deleted.rows_affected())
}
