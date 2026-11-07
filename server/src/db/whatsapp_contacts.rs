use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

pub struct Contact {
    pub jid: String,
    /// From the contact itself, or else from the number WhatsApp maps its LID to
    pub phone_number: Option<String>,
    /// Sealed [`crate::whatsapp::contacts::Names`]
    pub names: Vec<u8>,
}

pub struct Book {
    pub account_id: Uuid,
    pub imported: bool,
    pub contacts: Vec<Contact>,
}

pub async fn save(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    jid: &str,
    phone_number: Option<&str>,
    lid: Option<&str>,
    names: &[u8],
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO whatsapp_contacts (account_id, jid, phone_number, lid, names) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (account_id, jid) DO UPDATE \
         SET phone_number = EXCLUDED.phone_number, lid = EXCLUDED.lid, names = EXCLUDED.names, \
             updated_at = now()",
        account_id,
        jid,
        phone_number,
        lid,
        names,
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn remove(db: impl PgExecutor<'_>, account_id: Uuid, jid: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM whatsapp_contacts WHERE account_id = $1 AND jid = $2",
        account_id,
        jid
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn clear(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM whatsapp_contacts WHERE account_id = $1",
        account_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn mark_imported(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts SET contacts_imported_at = now() WHERE id = $1",
        account_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn forget_import(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts SET contacts_imported_at = NULL WHERE id = $1",
        account_id
    )
    .execute(db)
    .await?;

    Ok(())
}

/// False too once the account is gone, since there's nothing left to import into
pub async fn needs_import(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<bool> {
    let missing = sqlx::query_scalar!(
        r#"SELECT contacts_imported_at IS NULL AS "missing!" FROM whatsapp_accounts WHERE id = $1"#,
        account_id
    )
    .fetch_optional(db)
    .await?;

    Ok(missing.unwrap_or(false))
}

pub async fn is_saved(db: impl PgExecutor<'_>, user_id: Uuid, jid: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM whatsapp_contacts c
               JOIN whatsapp_accounts a ON a.id = c.account_id
               WHERE a.user_id = $1 AND c.jid = $2
           ) AS "saved!""#,
        user_id,
        jid
    )
    .fetch_one(db)
    .await
}

pub struct Found {
    pub account_id: Uuid,
    pub jid: String,
    pub names: Vec<u8>,
}

/// The saved contact with this number, by its own number or the one its LID maps to
pub async fn find_by_number(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    phone_number: &str,
) -> sqlx::Result<Option<Found>> {
    sqlx::query_as!(
        Found,
        r#"SELECT c.account_id, c.jid AS "jid!", c.names AS "names!"
           FROM whatsapp_contacts c
           JOIN whatsapp_accounts a ON a.id = c.account_id
           LEFT JOIN whatsapp_lid_mappings m ON m.account_id = c.account_id AND m.lid = c.lid
           WHERE a.user_id = $1 AND COALESCE(c.phone_number, m.phone_number) = $2
           ORDER BY c.jid LIMIT 1"#,
        user_id,
        phone_number
    )
    .fetch_optional(db)
    .await
}

/// The user's address book, if they have an account
pub async fn book(db: &PgPool, user_id: Uuid) -> sqlx::Result<Option<Book>> {
    let Some(account) = sqlx::query!(
        r#"SELECT id, contacts_imported_at IS NOT NULL AS "imported!"
           FROM whatsapp_accounts WHERE user_id = $1"#,
        user_id
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(None);
    };

    let contacts = sqlx::query_as!(
        Contact,
        r#"SELECT c.jid AS "jid!", COALESCE(c.phone_number, m.phone_number) AS phone_number,
               c.names AS "names!"
           FROM whatsapp_contacts c
           LEFT JOIN whatsapp_lid_mappings m ON m.account_id = c.account_id AND m.lid = c.lid
           WHERE c.account_id = $1"#,
        account.id
    )
    .fetch_all(db)
    .await?;

    Ok(Some(Book {
        account_id: account.id,
        imported: account.imported,
        contacts,
    }))
}
