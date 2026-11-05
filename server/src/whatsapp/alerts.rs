use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use super::status::Alert;
use crate::{
    db,
    email::{Mailer, templates},
};

/// # Errors
/// The user can't be loaded, or `app_url` cannot be a base for links. Delivery isn't awaited, so a
/// mail server failure is only logged.
pub async fn send(
    pool: &PgPool,
    mailer: &Mailer,
    app_url: &Url,
    user_id: Uuid,
    alert: Alert,
) -> anyhow::Result<()> {
    let to = db::users::email(pool, user_id).await?;
    let email = templates::security_alert(alert.into(), app_url)?;
    mailer.send_in_background(to, email);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::PgPool;
    use url::Url;

    use super::{Alert, send};
    use crate::{db, email::Mailer};

    #[sqlx::test]
    async fn emails_the_accounts_owner(pool: PgPool) {
        let owner = db::users::insert(&pool, "ada@example.com", "unused", &[])
            .await
            .unwrap();
        db::users::insert(&pool, "grace@example.com", "unused", &[])
            .await
            .unwrap();
        let outbox = Arc::default();
        let mailer = Mailer::Outbox(Arc::clone(&outbox));
        let app_url = Url::parse("https://example.com/").unwrap();

        send(&pool, &mailer, &app_url, owner.id, Alert::UnlinkedFromApp)
            .await
            .unwrap();

        let sent: Vec<_> = outbox
            .lock()
            .unwrap()
            .iter()
            .map(|(to, email)| (to.clone(), email.subject))
            .collect();
        assert_eq!(
            sent,
            [(
                "ada@example.com".to_owned(),
                "WhatsApp was unlinked from your account"
            )]
        );
    }
}
