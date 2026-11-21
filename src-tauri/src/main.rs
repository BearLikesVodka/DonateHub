use clap::Parser;
use donatehub_core::app::build;
use donatehub_core::config::{data_dir_from_env, resolve_port};
use donatehub_core::server::Server;
use donatehub_core::store::Store;

#[derive(Parser)]
#[command(name = "donatehub", about = "DonateHub: цель донатов для OBS")]
struct Cli {
    /// только сервер, без окна
    #[arg(long)]
    headless: bool,
    /// запуститься свёрнутым в трей
    #[arg(long)]
    hidden: bool,
}

fn init_logging() {
    // HTTP clients may log full request URLs at debug level, and the DonateX hub URL carries the token.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,reqwest=warn,hyper=warn,hyper_util=warn,tungstenite=warn".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logging();
    let store = Store::open(&data_dir_from_env()?.join("app.db"))?;
    let port = resolve_port(&store, &|k| std::env::var(k).ok());
    if cli.headless {
        return run_headless(store, port);
    }
    let _ = cli.hidden;
    anyhow::bail!("окно пока не реализовано, запустите с --headless")
}

fn run_headless(store: Store, port: u16) -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let (router, state) = build(store, true, port);
        let server = Server::start(router, state.collectors.clone(), port).await?;
        println!("Настройки:  http://127.0.0.1:{port}/\nОверлей:    http://127.0.0.1:{port}/overlay");
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        server.stop().await;
        Ok(())
    })
}
