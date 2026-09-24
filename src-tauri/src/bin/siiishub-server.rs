//! siiishub-server: SIIISHUB in the browser. Configured through environment
//! variables, see docs/WEB.md.

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,siiishub_desktop_lib=debug".into()),
        )
        .with_target(false)
        .init();

    let config = siiishub_desktop_lib::server::Config::from_env()?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(siiishub_desktop_lib::server::run(config))
}
