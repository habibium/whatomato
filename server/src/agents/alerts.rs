use uuid::Uuid;

use crate::{
    db,
    email::templates::{self, AgentAlert},
    state::AppState,
};

/// Emails the owner. A failure is logged, never passed on: the change it reports has been made
/// already, and an access key in the same response must still reach the owner.
pub async fn send(state: &AppState, user_id: Uuid, alert: AgentAlert<'_>) {
    if let Err(error) = try_send(state, user_id, &alert).await {
        tracing::error!(%user_id, "couldn't email an agent alert: {error:#}");
    }
}

async fn try_send(state: &AppState, user_id: Uuid, alert: &AgentAlert<'_>) -> anyhow::Result<()> {
    let to = db::users::email(&state.pool, user_id).await?;
    let email = templates::agent_alert(alert, &state.app_url)?;
    state.mailer.send_in_background(to, email);
    Ok(())
}
