use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

mod dto;
mod handlers;
#[cfg(test)]
mod tests;

pub(super) use dto::check_body;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_messages, handlers::create_message))
        .routes(routes!(handlers::send_now))
        .routes(routes!(handlers::next_runs))
        .routes(routes!(handlers::preview))
        .routes(routes!(
            handlers::get_message,
            handlers::update_message,
            handlers::toggle_message,
            handlers::delete_message
        ))
}
