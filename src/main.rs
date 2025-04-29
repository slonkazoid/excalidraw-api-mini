use std::borrow::Cow;
use std::error::Error;
use std::net::SocketAddr;
use std::pin::Pin;
use std::str::FromStr;

use axum::body::{Body, to_bytes};
use axum::extract::rejection::LengthLimitError;
use axum::extract::{Path, State};
use axum::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL};
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::status::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, options, post};
use axum::{Json, Router};
use color_eyre::eyre::{self, Context, eyre};
use libslonk::trace_layer;
use serde_json::json;
use sqlx::PgPool;
use sqlx::types::Uuid;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::select;
use tokio::signal::unix::Signal;
use tower::limit::ConcurrencyLimitLayer;
use tracing::level_filters::LevelFilter;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use ulid::Ulid;

const CACHE_CONTROL_VALUE: HeaderValue = HeaderValue::from_static("max-age=31536000, immutable");
const CACHE_1Y: (HeaderName, HeaderValue) = (CACHE_CONTROL, CACHE_CONTROL_VALUE);
const MAX_UPLOAD: usize = 3 * 1024 * 1024;

const UPLOAD: &str = "INSERT INTO entries (id, value) VALUES ($1, $2)";
const RETRIEVE: &str = "SELECT id, value FROM entries WHERE id=$1";

#[derive(sqlx::FromRow, Debug)]
struct Retrieved {
    value: Vec<u8>,
}

#[derive(Clone, Debug)]
struct AppState {
    pub pool: PgPool,
    pub allow_origin: HeaderValue,
}

#[derive(Error, Debug)]
enum InternalError {
    #[error(transparent)]
    AxumError(#[from] axum::Error),
    #[error("error while contacting database: {0}")]
    Pgerror(#[from] sqlx::Error),
}

impl IntoResponse for InternalError {
    fn into_response(self) -> axum::response::Response {
        let error = self.to_string();
        error!("error while handling request: {error}");
        (StatusCode::INTERNAL_SERVER_ERROR, error).into_response()
    }
}

async fn handle_options(
    State(AppState { allow_origin, .. }): State<AppState>,
) -> Result<impl IntoResponse, InternalError> {
    Ok([(ACCESS_CONTROL_ALLOW_ORIGIN, allow_origin), CACHE_1Y])
}

async fn upload(
    State(AppState { pool, allow_origin }): State<AppState>,
    body: Body,
) -> Result<impl IntoResponse, InternalError> {
    let body = match to_bytes(body, MAX_UPLOAD).await {
        Ok(v) => v,
        Err(err) => {
            if err.source().is_some_and(|e| e.is::<LengthLimitError>()) {
                return Ok(Json(json!({
                    "error_class": "RequestTooLargeError"
                }))
                .into_response());
            } else {
                return Err(err.into());
            }
        }
    };

    let id = Ulid::new();
    sqlx::query(UPLOAD)
        .bind(Uuid::from(id))
        .bind(&*body)
        .execute(&pool)
        .await?;

    Ok((
        [(ACCESS_CONTROL_ALLOW_ORIGIN, allow_origin)],
        Json(json!({
            "id": id.to_string(),
        })),
    )
        .into_response())
}

async fn retrieve(
    Path(id): Path<String>,
    State(AppState { pool, allow_origin }): State<AppState>,
) -> Result<impl IntoResponse, InternalError> {
    let Ok(id) = Ulid::from_str(&id) else {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    };
    let row = match sqlx::query_as(RETRIEVE)
        .bind(Uuid::from(id))
        .fetch_one(&pool)
        .await
    {
        Ok(v) => Ok(Some(v)),
        Err(err) => match err {
            sqlx::Error::RowNotFound => Ok(None),
            _ => Err(err),
        },
    }?;

    match row {
        Some(Retrieved { value, .. }) => Ok((
            [(ACCESS_CONTROL_ALLOW_ORIGIN, allow_origin), CACHE_1Y],
            value,
        )
            .into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_error::ErrorLayer::default())
        .init();
    color_eyre::install()?;

    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| eyre!("`DATABASE_URL` not set"))?;
    let allow_origin = HeaderValue::from_str(
        &std::env::var("CORS_ORIGIN")
            .map(Cow::Owned)
            .unwrap_or("*".into()),
    )
    .context("failed to parse `CORS_ORIGIN`")?;
    let socket_addr: SocketAddr = std::env::var("LISTEN")
        .map(Cow::Owned)
        .unwrap_or("[::]:2799".into())
        .parse()
        .context("failed to parse `LISTEN`")?;
    let max_concurrency = std::env::var("CONCURRENCY")
        .map(|v| v.parse().context("failed to parse `CONCURRENCY`"))
        .ok()
        .unwrap_or(Ok(100))?;

    let pool = PgPool::connect(&database_url)
        .await
        .context("failed to connect to database")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("failed to run migrations")?;

    let state = AppState { pool, allow_origin };

    let app = Router::new()
        .route("/", post(upload))
        .route("/{id}", get(retrieve))
        .fallback(options(handle_options))
        .layer(trace_layer!())
        .layer(ConcurrencyLimitLayer::new(max_concurrency))
        .with_state(state);

    let listener = TcpListener::bind(socket_addr)
        .await
        .with_context(|| format!("failed to listen on {socket_addr}"))?;
    let local_addr = listener.local_addr()?;

    info!("listening on http://{local_addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            // wanted to have a little bit of fun here :D
            let ctrl_c = tokio::signal::ctrl_c();
            let mut sigterm_handler =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
            let sigterm: Pin<Box<dyn Future<Output = Option<()>> + Send>> = sigterm_handler
                .as_mut()
                .map(Signal::recv)
                .map(|fut| Box::pin(fut) as _)
                .unwrap_or_else(|_| Box::pin(std::future::pending()) as _);
            select! {
                _ = sigterm => {},
                _ = ctrl_c => {}
            }
            info!("exiting…");
        })
        .await
        .context("failed to serve app")?;

    Ok(())
}
