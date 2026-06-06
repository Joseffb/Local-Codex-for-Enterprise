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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let config = EnterpriseConfig::from_runtime_parts(args.bind_addr, Some(args.database_url));
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
