use clap::{Args as ClapArgs, Parser, ValueEnum};
use tracing::{info, warn};

use aether_data::postgres::PostgresPoolConfig;
use aether_data::redis::RedisClientConfig;
use aether_gateway::{
    build_router_with_state, AppState, FrontdoorCorsConfig, FrontdoorUserRpmConfig,
    GatewayDataConfig, UsageRuntimeConfig, VideoTaskTruthSourceMode,
};
use aether_runtime::{
    init_service_runtime, DistributedConcurrencyGate, RedisDistributedConcurrencyConfig,
    ServiceRuntimeConfig,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum VideoTaskTruthSourceArg {
    PythonSyncReport,
    RustAuthoritative,
}

impl From<VideoTaskTruthSourceArg> for VideoTaskTruthSourceMode {
    fn from(value: VideoTaskTruthSourceArg) -> Self {
        match value {
            VideoTaskTruthSourceArg::PythonSyncReport => VideoTaskTruthSourceMode::PythonSyncReport,
            VideoTaskTruthSourceArg::RustAuthoritative => {
                VideoTaskTruthSourceMode::RustAuthoritative
            }
        }
    }
}

#[derive(ClapArgs, Debug, Clone)]
struct GatewayDataArgs {
    #[arg(long, env = "AETHER_GATEWAY_DATA_POSTGRES_URL")]
    postgres_url: Option<String>,

    #[arg(long, env = "AETHER_GATEWAY_DATA_ENCRYPTION_KEY")]
    encryption_key: Option<String>,

    #[arg(long, env = "AETHER_GATEWAY_DATA_REDIS_URL")]
    redis_url: Option<String>,

    #[arg(long, env = "AETHER_GATEWAY_DATA_REDIS_KEY_PREFIX")]
    redis_key_prefix: Option<String>,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_MIN_CONNECTIONS",
        default_value_t = 1
    )]
    postgres_min_connections: u32,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_MAX_CONNECTIONS",
        default_value_t = 20
    )]
    postgres_max_connections: u32,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_ACQUIRE_TIMEOUT_MS",
        default_value_t = 5_000
    )]
    postgres_acquire_timeout_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_IDLE_TIMEOUT_MS",
        default_value_t = 60_000
    )]
    postgres_idle_timeout_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_MAX_LIFETIME_MS",
        default_value_t = 1_800_000
    )]
    postgres_max_lifetime_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_STATEMENT_CACHE_CAPACITY",
        default_value_t = 100
    )]
    postgres_statement_cache_capacity: usize,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DATA_POSTGRES_REQUIRE_SSL",
        default_value_t = false
    )]
    postgres_require_ssl: bool,
}

impl GatewayDataArgs {
    fn effective_postgres_url(&self) -> Option<String> {
        self.postgres_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                std::env::var("DATABASE_URL")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn effective_redis_url(&self) -> Option<String> {
        self.redis_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                std::env::var("REDIS_URL")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn effective_encryption_key(&self) -> Option<String> {
        self.encryption_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                std::env::var("ENCRYPTION_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn configured_encryption_key_mismatch(&self) -> bool {
        let gateway_value = std::env::var("AETHER_GATEWAY_DATA_ENCRYPTION_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let default_value = std::env::var("ENCRYPTION_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        matches!(
            (gateway_value, default_value),
            (Some(gateway_value), Some(default_value)) if gateway_value != default_value
        )
    }

    fn to_config(&self) -> GatewayDataConfig {
        let database_url = self.effective_postgres_url();
        let redis_url = self.effective_redis_url();

        let mut config = match database_url.as_deref() {
            Some(database_url) => GatewayDataConfig::from_postgres_config(PostgresPoolConfig {
                database_url: database_url.to_string(),
                min_connections: self.postgres_min_connections,
                max_connections: self.postgres_max_connections,
                acquire_timeout_ms: self.postgres_acquire_timeout_ms,
                idle_timeout_ms: self.postgres_idle_timeout_ms,
                max_lifetime_ms: self.postgres_max_lifetime_ms,
                statement_cache_capacity: self.postgres_statement_cache_capacity,
                require_ssl: self.postgres_require_ssl,
            }),
            None => GatewayDataConfig::disabled(),
        };

        if let Some(redis_url) = redis_url.as_deref() {
            config = config.with_redis_config(RedisClientConfig {
                url: redis_url.to_string(),
                key_prefix: self
                    .redis_key_prefix
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            });
        }

        match self.effective_encryption_key() {
            Some(value) => config.with_encryption_key(value),
            None => config,
        }
    }
}

#[derive(ClapArgs, Debug, Clone)]
struct GatewayUsageArgs {
    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_STREAM_KEY",
        default_value = "usage:events"
    )]
    queue_stream_key: String,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_GROUP",
        default_value = "usage_consumers"
    )]
    queue_group: String,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_DLQ_STREAM_KEY",
        default_value = "usage:events:dlq"
    )]
    queue_dlq_stream_key: String,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_STREAM_MAXLEN",
        default_value_t = 2_000
    )]
    queue_stream_maxlen: usize,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_BATCH_SIZE",
        default_value_t = 200
    )]
    queue_batch_size: usize,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_BLOCK_MS",
        default_value_t = 500
    )]
    queue_block_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_RECLAIM_IDLE_MS",
        default_value_t = 30_000
    )]
    queue_reclaim_idle_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_RECLAIM_COUNT",
        default_value_t = 200
    )]
    queue_reclaim_count: usize,

    #[arg(
        long,
        env = "AETHER_GATEWAY_USAGE_QUEUE_RECLAIM_INTERVAL_MS",
        default_value_t = 5_000
    )]
    queue_reclaim_interval_ms: u64,
}

impl GatewayUsageArgs {
    fn to_config(&self) -> UsageRuntimeConfig {
        UsageRuntimeConfig {
            enabled: true,
            stream_key: self.queue_stream_key.trim().to_string(),
            consumer_group: self.queue_group.trim().to_string(),
            dlq_stream_key: self.queue_dlq_stream_key.trim().to_string(),
            stream_maxlen: self.queue_stream_maxlen.max(1),
            consumer_batch_size: self.queue_batch_size.max(1),
            consumer_block_ms: self.queue_block_ms.max(1),
            reclaim_idle_ms: self.queue_reclaim_idle_ms.max(1),
            reclaim_count: self.queue_reclaim_count.max(1),
            reclaim_interval_ms: self.queue_reclaim_interval_ms.max(1),
        }
    }
}

#[derive(ClapArgs, Debug, Clone)]
struct GatewayFrontdoorArgs {
    #[arg(long, env = "ENVIRONMENT", default_value = "development")]
    environment: String,

    #[arg(long, env = "CORS_ORIGINS")]
    cors_origins: Option<String>,

    #[arg(long, env = "CORS_ALLOW_CREDENTIALS", default_value_t = true)]
    cors_allow_credentials: bool,
}

impl GatewayFrontdoorArgs {
    fn cors_config(&self) -> Option<FrontdoorCorsConfig> {
        FrontdoorCorsConfig::from_environment(
            self.environment.trim(),
            self.cors_origins.as_deref(),
            self.cors_allow_credentials,
        )
    }
}

#[derive(ClapArgs, Debug, Clone)]
struct GatewayRateLimitArgs {
    #[arg(long, env = "RPM_BUCKET_SECONDS", default_value_t = 60)]
    bucket_seconds: u64,

    #[arg(long, env = "RPM_KEY_TTL_SECONDS", default_value_t = 120)]
    key_ttl_seconds: u64,

    #[arg(long, env = "RATE_LIMIT_FAIL_OPEN", default_value_t = true)]
    fail_open: bool,
}

impl GatewayRateLimitArgs {
    fn config(&self) -> FrontdoorUserRpmConfig {
        FrontdoorUserRpmConfig::new(self.bucket_seconds, self.key_ttl_seconds, self.fail_open)
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "aether-gateway",
    about = "Phase 3a Rust ingress gateway for Aether"
)]
struct Args {
    #[arg(long, env = "AETHER_GATEWAY_BIND", default_value = "0.0.0.0:80")]
    bind: String,

    /// Path to frontend static files directory (SPA). When set, the gateway
    /// serves the frontend directly without nginx.
    #[arg(long, env = "AETHER_GATEWAY_STATIC_DIR")]
    static_dir: Option<String>,

    #[arg(
        long,
        env = "AETHER_GATEWAY_VIDEO_TASK_TRUTH_SOURCE_MODE",
        value_enum,
        default_value = "python-sync-report"
    )]
    video_task_truth_source_mode: VideoTaskTruthSourceArg,

    #[arg(
        long,
        env = "AETHER_GATEWAY_VIDEO_TASK_POLLER_INTERVAL_MS",
        default_value_t = 5000
    )]
    video_task_poller_interval_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_VIDEO_TASK_POLLER_BATCH_SIZE",
        default_value_t = 32
    )]
    video_task_poller_batch_size: usize,

    #[arg(long, env = "AETHER_GATEWAY_VIDEO_TASK_STORE_PATH")]
    video_task_store_path: Option<String>,

    #[arg(long, env = "AETHER_GATEWAY_MAX_IN_FLIGHT_REQUESTS")]
    max_in_flight_requests: Option<usize>,

    #[arg(long, env = "AETHER_GATEWAY_DISTRIBUTED_REQUEST_LIMIT")]
    distributed_request_limit: Option<usize>,

    #[arg(long, env = "AETHER_GATEWAY_DISTRIBUTED_REQUEST_REDIS_URL")]
    distributed_request_redis_url: Option<String>,

    #[arg(long, env = "AETHER_GATEWAY_DISTRIBUTED_REQUEST_REDIS_KEY_PREFIX")]
    distributed_request_redis_key_prefix: Option<String>,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DISTRIBUTED_REQUEST_LEASE_TTL_MS",
        default_value_t = 30_000
    )]
    distributed_request_lease_ttl_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DISTRIBUTED_REQUEST_RENEW_INTERVAL_MS",
        default_value_t = 10_000
    )]
    distributed_request_renew_interval_ms: u64,

    #[arg(
        long,
        env = "AETHER_GATEWAY_DISTRIBUTED_REQUEST_COMMAND_TIMEOUT_MS",
        default_value_t = 1_000
    )]
    distributed_request_command_timeout_ms: u64,

    #[command(flatten)]
    data: GatewayDataArgs,

    #[command(flatten)]
    usage: GatewayUsageArgs,

    #[command(flatten)]
    frontdoor: GatewayFrontdoorArgs,

    #[command(flatten)]
    rate_limit: GatewayRateLimitArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_service_runtime(ServiceRuntimeConfig::new(
        "aether-gateway",
        "aether_gateway=info",
    ))?;

    let args = Args::parse();
    let data_postgres_url = args.data.effective_postgres_url();
    let data_redis_url = args.data.effective_redis_url();
    let data_config = args.data.to_config();
    if args.data.configured_encryption_key_mismatch() {
        warn!(
            "AETHER_GATEWAY_DATA_ENCRYPTION_KEY differs from ENCRYPTION_KEY; aether-gateway will prefer the gateway-specific value"
        );
    }
    info!(
        bind = %args.bind,
        environment = %args.frontdoor.environment,
        frontdoor_mode = "compatibility_frontdoor",
        static_dir = args.static_dir.as_deref().unwrap_or("-"),
        cors_origins = args.frontdoor.cors_origins.as_deref().unwrap_or("-"),
        cors_allow_credentials = args.frontdoor.cors_allow_credentials,
        frontdoor_rpm_bucket_seconds = args.rate_limit.bucket_seconds,
        frontdoor_rpm_key_ttl_seconds = args.rate_limit.key_ttl_seconds,
        frontdoor_rpm_fail_open = args.rate_limit.fail_open,
        video_task_truth_source_mode = ?args.video_task_truth_source_mode,
        video_task_poller_interval_ms = args.video_task_poller_interval_ms,
        video_task_poller_batch_size = args.video_task_poller_batch_size,
        video_task_store_path = args.video_task_store_path.as_deref().unwrap_or("-"),
        max_in_flight_requests = args.max_in_flight_requests.unwrap_or_default(),
        distributed_request_limit = args.distributed_request_limit.unwrap_or_default(),
        distributed_request_redis_url = args
            .distributed_request_redis_url
            .as_deref()
            .unwrap_or("-"),
        data_postgres_url = data_postgres_url.as_deref().unwrap_or("-"),
        data_redis_url = data_redis_url.as_deref().unwrap_or("-"),
        data_has_encryption_key = data_config.encryption_key().is_some(),
        data_postgres_require_ssl = args.data.postgres_require_ssl,
        "aether-gateway started"
    );

    let mut state = AppState::new()?
        .with_data_config(data_config)?
        .with_usage_runtime_config(args.usage.to_config())?
        .with_video_task_truth_source_mode(args.video_task_truth_source_mode.into());
    if let Some(cors_config) = args.frontdoor.cors_config() {
        state = state.with_frontdoor_cors_config(cors_config);
    }
    state = state.with_frontdoor_user_rpm_config(args.rate_limit.config());
    if matches!(
        args.video_task_truth_source_mode,
        VideoTaskTruthSourceArg::RustAuthoritative
    ) {
        state = state.with_video_task_poller_config(
            std::time::Duration::from_millis(args.video_task_poller_interval_ms.max(1)),
            args.video_task_poller_batch_size.max(1),
        );
    }
    if let Some(path) = args
        .video_task_store_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state = state.with_video_task_store_path(path)?;
    }
    if let Some(limit) = args.max_in_flight_requests.filter(|limit| *limit > 0) {
        state = state.with_request_concurrency_limit(limit);
    }
    if let Some(limit) = args.distributed_request_limit.filter(|limit| *limit > 0) {
        let redis_url = args
            .distributed_request_redis_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "AETHER_GATEWAY_DISTRIBUTED_REQUEST_REDIS_URL is required when distributed request limit is enabled",
                )
            })?;
        state =
            state.with_distributed_request_concurrency_gate(DistributedConcurrencyGate::new_redis(
                "gateway_requests_distributed",
                limit,
                RedisDistributedConcurrencyConfig {
                    url: redis_url.to_string(),
                    key_prefix: args
                        .distributed_request_redis_key_prefix
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                    lease_ttl_ms: args.distributed_request_lease_ttl_ms.max(1),
                    renew_interval_ms: args.distributed_request_renew_interval_ms.max(1),
                    command_timeout_ms: Some(args.distributed_request_command_timeout_ms.max(1)),
                },
            )?);
    }
    if !state.has_usage_data_writer() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage persistence requires a configured Postgres data backend; set AETHER_GATEWAY_DATA_POSTGRES_URL before starting aether-gateway",
        )
        .into());
    }
    info!(
        has_data_backends = state.has_data_backends(),
        has_video_task_data_reader = state.has_video_task_data_reader(),
        has_usage_data_writer = state.has_usage_data_writer(),
        has_usage_worker_backend = state.has_usage_worker_backend(),
        control_api_configured = true,
        execution_runtime_configured = state.execution_runtime_configured(),
        "aether-gateway data layer configured"
    );
    // Run pending database migrations before serving traffic
    info!("running database migrations...");
    if state.run_postgres_migrations().await? {
        info!("database migrations complete");
    }

    let background_tasks = state.spawn_background_tasks();
    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    let api_router = build_router_with_state(state);

    // Compose the final router: API routes + optional static file serving + CF header stripping
    let router = if let Some(ref static_dir) = args.static_dir {
        use tower_http::compression::CompressionLayer;
        use tower_http::services::{ServeDir, ServeFile};

        let static_path = std::path::PathBuf::from(static_dir);
        let index_html = static_path.join("index.html");
        info!(static_dir = %static_dir, "serving frontend static files");

        // ServeDir with SPA fallback: if no static file matches, serve index.html
        let serve_dir = ServeDir::new(&static_path).not_found_service(ServeFile::new(&index_html));

        api_router
            .fallback_service(serve_dir)
            .layer(CompressionLayer::new())
            .layer(axum::middleware::from_fn(
                aether_gateway::strip_cf_headers_middleware,
            ))
    } else {
        api_router.layer(axum::middleware::from_fn(
            aether_gateway::strip_cf_headers_middleware,
        ))
    };

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    for handle in background_tasks {
        handle.abort();
    }
    Ok(())
}
