use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

mod dto;
mod handlers;
mod sign_in;
#[cfg(test)]
mod tests;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_agents, handlers::create_agent))
        .routes(routes!(handlers::list_agent_presets))
        .routes(routes!(handlers::pause_all_agents))
        .routes(routes!(handlers::list_agent_approvals))
        .routes(routes!(handlers::get_agent_approval))
        .routes(routes!(handlers::list_agent_activity))
        .routes(routes!(handlers::approve_agent_request))
        .routes(routes!(handlers::reject_agent_request))
        .routes(routes!(
            handlers::get_agent,
            handlers::update_agent,
            handlers::revoke_agent
        ))
        .routes(routes!(handlers::pause_agent))
        .routes(routes!(handlers::resume_agent))
        .routes(routes!(handlers::rotate_agent_key))
        .routes(routes!(sign_in::check_agent_sign_in))
        .routes(routes!(sign_in::approve_agent_sign_in))
}
