use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

mod dto;
mod handlers;
#[cfg(test)]
mod tests;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::get_status))
        .routes(routes!(handlers::link))
        .routes(routes!(handlers::cancel_link))
        .routes(routes!(handlers::pause))
        .routes(routes!(handlers::resume))
        .routes(routes!(handlers::unlink))
        .routes(routes!(handlers::groups))
        .routes(routes!(handlers::contacts))
        .routes(routes!(handlers::picture))
        .routes(routes!(handlers::send_test_message))
}
