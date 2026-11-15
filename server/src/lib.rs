pub mod agents;
mod auth;
pub mod config;
pub mod crypto;
mod csrf;
mod db;
pub mod email;
mod error;
mod extract;
pub mod mcp;
mod rate_limit;
mod routes;
pub mod scheduler;
pub mod state;
pub mod whatsapp;

use axum::{Router, middleware, routing::any};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;
pub use crate::{
    auth::pwned::PwnedPasswords,
    db::spawn_cleanup,
    rate_limit::{ClientIpHeader, RateLimits},
};

#[derive(OpenApi)]
#[openapi(info(title = "Whatomato"))]
struct ApiDoc;

pub fn router() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(
            "/api",
            OpenApiRouter::new()
                .merge(routes::health::router())
                // On the whole nest, so routes added to it later are limited too
                .nest(
                    "/auth",
                    routes::auth::router().layer(rate_limit::client_ip_layer()),
                )
                .nest(
                    "/whatsapp",
                    routes::whatsapp::router().layer(rate_limit::client_ip_layer()),
                )
                .nest(
                    "/messages",
                    routes::messages::router().layer(rate_limit::client_ip_layer()),
                )
                .nest(
                    "/templates",
                    routes::templates::router().layer(rate_limit::client_ip_layer()),
                )
                .nest("/deliveries", routes::deliveries::router())
                .nest(
                    "/agents",
                    routes::agents::router().layer(rate_limit::client_ip_layer()),
                )
                // on the nested router so unknown /api paths 404 instead of falling
                // through to the SPA fallback in main.rs
                .fallback(routes::not_found)
                .layer(middleware::from_fn(csrf::reject_cross_site_writes)),
        )
        // that fallback's catch-all needs a non-empty suffix, so bare "/api/" misses it
        .route("/api/", any(routes::not_found))
        .split_for_parts();
    (oauth_discovery_not_found(router), api)
}

/// MCP clients that get a 401 look for OAuth metadata here. There's no OAuth yet, and the SPA
/// fallback's HTML answering 200 reads to them as broken JSON.
fn oauth_discovery_not_found(mut router: Router<AppState>) -> Router<AppState> {
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
    ] {
        router = router
            .route(path, any(routes::not_found))
            .route(&format!("{path}/{{*suffix}}"), any(routes::not_found));
    }
    router
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
    };
    use serde_json::{Value, json};
    use sqlx::PgPool;
    use tower::ServiceExt;

    use crate::routes::auth::tests::TestApp;

    #[sqlx::test]
    async fn oauth_discovery_is_not_found_rather_than_the_web_app(pool: PgPool) {
        let app = TestApp::new(pool);
        let router = crate::router()
            .0
            .fallback(|| async { "the web app" })
            .with_state(app.state.clone());
        let send = |method: Method, path: String| {
            let router = router.clone();
            async move {
                let request = Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap();
                let response = router.oneshot(request).await.unwrap();
                let status = response.status();
                let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
                let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                (status, content_type, bytes)
            }
        };

        for base in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server",
            "/.well-known/openid-configuration",
        ] {
            for path in [base.to_owned(), format!("{base}/mcp")] {
                for method in [Method::GET, Method::POST] {
                    let (status, content_type, bytes) = send(method, path.clone()).await;
                    assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
                    assert_eq!(content_type.unwrap(), "application/json", "{path}");
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(body, json!({ "error": "Not Found" }), "{path}");
                }
            }
        }

        let (status, _, bytes) = send(Method::GET, "/.well-known/security.txt".into()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&bytes[..], b"the web app");
    }
}
