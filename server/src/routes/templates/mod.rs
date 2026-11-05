use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

mod dto;
mod handlers;
#[cfg(test)]
mod tests;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_templates, handlers::create_template))
        .routes(routes!(
            handlers::get_template,
            handlers::update_template,
            handlers::delete_template
        ))
}
