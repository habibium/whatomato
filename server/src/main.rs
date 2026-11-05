use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use axum::{
    Extension, Router,
    extract::Request,
    http::{HeaderValue, header},
    response::Response,
};
use server::{
    ClientIpHeader, PwnedPasswords, RateLimits,
    config::Config,
    crypto::Cipher,
    email::Mailer,
    scheduler::Scheduler,
    state::AppState,
    whatsapp::{manager::WhatsApp, supervisor::Context},
};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tower_http::{
    compression::{
        CompressionLayer, CompressionLevel,
        predicate::{DefaultPredicate, NotForContentType, Predicate},
    },
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use utoipa_scalar::{Scalar, Servable as ScalarServable};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    // .env is a dev convenience; production supplies real environment variables
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;

    let pool = PgPoolOptions::new().connect(&config.database_url).await?;
    sqlx::migrate!().run(&pool).await?;
    server::spawn_cleanup(pool.clone());

    let mailer = Mailer::new(config.smtp_url.as_deref(), config.email_from)?;
    tokio::spawn({
        let mailer = mailer.clone();
        async move { mailer.check().await }
    });
    tracing::info!("links in emails point at {}", config.app_url);
    let cipher = Cipher::new(&config.encryption_key);
    let app_url = Arc::new(config.app_url);
    let whatsapp = WhatsApp::new(Context::new(
        pool.clone(),
        cipher.clone(),
        mailer.clone(),
        Arc::clone(&app_url),
    ));
    whatsapp.boot().await?;
    whatsapp.refresh_version_daily();
    let scheduler = Scheduler::spawn(pool.clone(), whatsapp.clone());
    let state = AppState {
        pool: pool.clone(),
        mailer,
        app_url,
        rate_limits: RateLimits::default(),
        pwned_passwords: PwnedPasswords::hibp()?,
        cipher,
        whatsapp: whatsapp.clone(),
    };

    let (router, api) = server::router();

    // `precompressed_*` serves the .br/.gz siblings the web build emits, so the
    // CompressionLayer below only ever has to compress API and docs responses.
    let index = ServeFile::new(config.web_dir.join("index.html"))
        .precompressed_br()
        .precompressed_gzip();
    let web = ServeDir::new(&config.web_dir)
        .precompressed_br()
        .precompressed_gzip()
        .fallback(index);
    let assets = ServeDir::new(config.web_dir.join("assets"))
        .precompressed_br()
        .precompressed_gzip();

    // Only /assets/* filenames are content-hashed, so nothing else may be cached;
    // a 404 there must not be pinned either, or a mid-deploy miss sticks for a year.
    let assets_router =
        Router::new()
            .fallback_service(assets)
            .layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                |res: &Response| {
                    res.status()
                        .is_success()
                        .then(|| HeaderValue::from_static("public, max-age=31536000, immutable"))
                },
            ));
    let web_router = Router::new()
        .fallback_service(web)
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ));

    let app = router
        .fallback_service(web_router)
        .with_state(state)
        .merge(Scalar::with_url("/api/docs", api))
        .nest_service("/assets", assets_router)
        .layer(Extension(ClientIpHeader(config.client_ip_header)))
        .layer(
            CompressionLayer::new()
                .br(true)
                .gzip(true)
                .quality(CompressionLevel::Fastest)
                // woff2 is already compressed; encoding it only adds bytes
                .compress_when(DefaultPredicate::new().and(NotForContentType::const_new("font/"))),
        )
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                // Path only, as query strings carry the tokens from emailed links. tower-http's own
                // target keeps RUST_LOG filtering this span as before.
                tracing::debug_span!(
                    target: "tower_http::trace::make_span",
                    "request",
                    method = %request.method(),
                    path = request.uri().path(),
                )
            }),
        );

    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.port)).await?;
    tracing::info!("server running on http://{}", listener.local_addr()?);

    // The peer address is what the rate limit keys on when no CLIENT_IP_HEADER is set
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Sends finish on their clients, and clients flush their keys to the database, so they stop in
    // this order before the pool closes
    scheduler.shutdown().await;
    whatsapp.shutdown().await;
    pool.close().await;
    tracing::info!("server stopped");
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            // Libraries stay at warn: the WhatsApp library logs phone numbers and pair codes at info
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,server=debug,tower_http=debug")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Resolves on Ctrl-C, or on the SIGTERM that `docker stop` sends. As PID 1 in the container the
/// server gets no default SIGTERM handling, so without this Docker kills it once its timeout runs out.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install the Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install the SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down, letting in-flight requests finish");
}
