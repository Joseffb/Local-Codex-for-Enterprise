use anyhow::Context;
use clap::Parser;
use codex_enterprise_server::api;
use codex_enterprise_server::config::EnterpriseConfig;
use codex_enterprise_server::storage::PostgresEnterpriseStore;
use sqlx::postgres::PgPoolOptions;

#[derive(Debug, Parser)]
#[command(name = "local-codex-enterprise-server")]
#[command(about = "Self-hosted Local Codex enterprise control plane")]
struct Args {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    #[arg(
        long,
        env = "LOCAL_CODEX_ENTERPRISE_BIND",
        default_value = "127.0.0.1:8787"
    )]
    bind_addr: String,

    #[arg(long, env = "LOCAL_CODEX_ENTERPRISE_WORKER_COMMAND")]
    worker_command: Option<String>,

    #[arg(long = "worker-arg", env = "LOCAL_CODEX_ENTERPRISE_WORKER_ARGS")]
    worker_args: Vec<String>,

    #[arg(long, env = "LOCAL_CODEX_ENTERPRISE_WORKER_SOCKET_DIR")]
    worker_socket_dir: Option<String>,

    #[arg(long, env = "LOCAL_CODEX_ENTERPRISE_WORKER_LOG_DIR")]
    worker_log_dir: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let mut config = EnterpriseConfig::from_runtime_parts(args.bind_addr, Some(args.database_url));
    if let Some(worker_command) = args.worker_command {
        config.worker_command = worker_command;
    }
    if !args.worker_args.is_empty() {
        config.worker_args = args.worker_args;
    }
    if let Some(worker_socket_dir) = args.worker_socket_dir {
        config.worker_socket_dir = worker_socket_dir;
    }
    if let Some(worker_log_dir) = args.worker_log_dir {
        config.worker_log_dir = worker_log_dir;
    }
    let database_url = config
        .database_url
        .clone()
        .context("DATABASE_URL is required")?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("connect to postgres")?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("run enterprise migrations")?;

    let bind_addr = config.bind_addr.clone();
    let router = api::build_router_with_store(PostgresEnterpriseStore::new(pool), config);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("bind enterprise server on {bind_addr}"))?;

    axum::serve(listener, router)
        .await
        .context("serve enterprise api")?;
    Ok(())
}
