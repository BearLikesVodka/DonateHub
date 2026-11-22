// Release builds on Windows are GUI apps (no console window); `--headless` attaches to the caller's console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod clipboard;
mod desktop;
mod notify;
mod tray;
mod updates;

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
    let fmt = tracing_subscriber::fmt().with_env_filter(filter);
    #[cfg(windows)]
    if let Some(file) = windows_log_file() {
        // A GUI-subsystem app has no stdout: keep the log next to the data so problems can be diagnosed.
        fmt.with_ansi(false).with_writer(std::sync::Mutex::new(file)).init();
        return;
    }
    fmt.init();
}

/// `%APPDATA%\DonateHub\donatehub.log` for this run; the previous run's log is kept as `donatehub.old.log`.
#[cfg(windows)]
fn windows_log_file() -> Option<std::fs::File> {
    let dir = donatehub_core::config::windows_data_dir(&|k| std::env::var(k).ok()).ok()?;
    let log = dir.join("donatehub.log");
    let _ = std::fs::rename(&log, dir.join("donatehub.old.log"));
    std::fs::File::create(log).ok()
}

fn main() -> anyhow::Result<()> {
    // Before parsing: `--help`, argument errors and `--headless` output must reach cmd/PowerShell.
    #[cfg(windows)]
    attach_parent_console();
    let cli = Cli::parse();
    init_logging();
    let store = Store::open(&data_dir_from_env()?.join("app.db"))?;
    let port = resolve_port(&store, &|k| std::env::var(k).ok());
    if cli.headless {
        return run_headless(store, port);
    }
    std::process::exit(desktop::run(store, port, cli.hidden)?)
}

fn run_headless(store: Store, port: u16) -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let (router, state) = build(store, true, port);
        let server = Server::start(router, state.collectors.clone(), port).await?;
        println!("Настройки:  http://127.0.0.1:{port}/\nОверлей:    http://127.0.0.1:{port}/overlay");
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await?;
        server.stop().await;
        Ok(())
    })
}

/// A `windows_subsystem = "windows"` binary has no console; reuse the one of cmd/PowerShell that started it.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call without pointers; failure (no parent console) only means output goes nowhere.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
