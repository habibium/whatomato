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
pub mod oauth;
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
    (oauth::routes(router), api)
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
    async fn oauth_discovery_never_answers_with_the_web_app(pool: PgPool) {
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

        for path in [
            "/.well-known/openid-configuration",
            "/.well-known/openid-configuration/mcp",
            "/.well-known/oauth-authorization-server/mcp",
            "/.well-known/oauth-protected-resource/other",
        ] {
            for method in [Method::GET, Method::POST] {
                let (status, content_type, bytes) = send(method, path.to_owned()).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
                assert_eq!(content_type.unwrap(), "application/json", "{path}");
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(body, json!({ "error": "Not Found" }), "{path}");
            }
        }
        // The two this server does publish answer as JSON too
        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server",
        ] {
            let (status, content_type, _) = send(Method::GET, path.to_owned()).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(content_type.unwrap(), "application/json", "{path}");
        }

        let (status, _, bytes) = send(Method::GET, "/.well-known/security.txt".into()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&bytes[..], b"the web app");
    }
}
