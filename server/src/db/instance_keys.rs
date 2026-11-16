use sqlx::PgExecutor;

pub async fn find(db: impl PgExecutor<'_>, name: &str) -> sqlx::Result<Option<Vec<u8>>> {
    sqlx::query_scalar!("SELECT sealed FROM instance_keys WHERE name = $1", name)
        .fetch_optional(db)
        .await
}

/// Keeps what is there already, should two servers start at the same moment
pub async fn insert_if_absent(
    db: impl PgExecutor<'_>,
    name: &str,
    sealed: &[u8],
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO instance_keys (name, sealed) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING",
        name,
        sealed
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn replace(db: impl PgExecutor<'_>, name: &str, sealed: &[u8]) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE instance_keys SET sealed = $2, created_at = now() WHERE name = $1",
        name,
        sealed
    )
    .execute(db)
    .await?;
    Ok(())
}
