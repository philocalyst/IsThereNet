use istherenet::config::{self, Config};
use istherenet::ping::Pinger;
use istherenet::platform::{color_for_status, Overlay};
use istherenet::state::{Debouncer, NetStatus, NetStatusKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use tracing_subscriber::prelude::*;

fn setup_logging() {
    let log_dir = dirs::home_dir().expect("no home directory").join(".logs");
    std::fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::never(&log_dir, "istherenet.log");

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
                    "%Y-%m-%dT%H:%M:%S%.3fZ".to_string(),
                )),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
                    "%Y-%m-%dT%H:%M:%S%.3fZ".to_string(),
                ))
                .with_writer(file_appender),
        )
        .init();
}

fn run_shell_command(command: &str, status: &NetStatus) {
    let status_str = status.kind().to_string();
    let ping_time = format!("{}", status.rtt_ms().round() as i64);

    let shell = if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "/bin/sh"
    };

    let shell_flag = if cfg!(target_os = "windows") {
        "/C"
    } else {
        "-c"
    };

    match std::process::Command::new(shell)
        .arg(shell_flag)
        .arg(command)
        .env("STATUS", &status_str)
        .env("PING_TIME", &ping_time)
        .spawn()
    {
        Ok(_) => tracing::info!("Running shell command: {command}"),
        Err(error) => tracing::error!("Failed to run shell command: {error}"),
    }
}

fn start_config_watcher(
    config_path: PathBuf,
    config: Arc<Mutex<Config>>,
) -> Result<RecommendedWatcher, notify::Error> {
    let watch_dir = config_path
        .parent()
        .expect("config path must have parent")
        .to_path_buf();

    let config_filename = config_path
        .file_name()
        .expect("config path must have filename")
        .to_os_string();

    let mut watcher = notify::recommended_watcher(move |result: Result<notify::Event, _>| {
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                tracing::error!("Config watcher error: {error}");
                return;
            }
        };

        let is_config_change = event.paths.iter().any(|path| {
            path.file_name()
                .map(|name| name == config_filename)
                .unwrap_or(false)
        });

        if !is_config_change {
            return;
        }

        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {}
            _ => return,
        }

        match config::load_config() {
            Ok(new_config) => {
                let mut current = match config.lock() {
                    Ok(guard) => guard,
                    Err(error) => {
                        tracing::error!("Failed to lock config: {error}");
                        return;
                    }
                };
                if *current != new_config {
                    tracing::info!("Config updated: {new_config:?}");
                    *current = new_config;
                }
            }
            Err(error) => tracing::error!("Failed to reload config: {error}"),
        }
    })?;

    watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

fn main() {
    setup_logging();

    let initial_config = match config::load_config() {
        Ok(config) => {
            tracing::info!("Config loaded: {config:?}");
            config
        }
        Err(error) => {
            tracing::warn!("Failed to load config, using defaults: {error}");
            Config::default()
        }
    };

    let config = Arc::new(Mutex::new(initial_config));

    let config_path = config::config_path();
    let _watcher = match start_config_watcher(config_path, Arc::clone(&config)) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            tracing::error!("Failed to start config watcher: {error}");
            None
        }
    };

    #[cfg(target_os = "macos")]
    {
        let (sender, receiver) = mpsc::channel();
        let mut overlay = istherenet::platform_macos::MacOverlay::new(sender);

        let config_for_thread = Arc::clone(&config);
        std::thread::spawn(move || {
            run_ping_loop(config_for_thread, &mut overlay);
        });

        // Main thread runs the AppKit event loop
        istherenet::platform_macos::run_main_thread_loop(receiver);
    }

    #[cfg(target_os = "linux")]
    {
        let mut overlay = istherenet::platform_linux::LinuxOverlay::new();
        run_ping_loop(Arc::clone(&config), &mut overlay);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let mut overlay = istherenet::platform::create_overlay();
        run_ping_loop(Arc::clone(&config), overlay.as_mut());
    }
}

fn run_ping_loop(config: Arc<Mutex<Config>>, overlay: &mut dyn Overlay) {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!("Failed to create tokio runtime: {error}");
            return;
        }
    };

    runtime.block_on(async {
        let mut debouncer = Debouncer::new();
        let mut current_status: Option<NetStatusKind> = None;
        let mut last_config_snapshot = Config::default();

        loop {
            let config_snapshot = match config.lock() {
                Ok(guard) => guard.clone(),
                Err(error) => {
                    tracing::error!("Failed to lock config: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            if config_snapshot != last_config_snapshot {
                debouncer.reset();
                current_status = None;
                last_config_snapshot = config_snapshot.clone();
            }

            let target: IpAddr = match config_snapshot.ping_ip.parse() {
                Ok(ip) => ip,
                Err(error) => {
                    tracing::error!("Invalid ping IP '{}': {error}", config_snapshot.ping_ip);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let mut pinger = match Pinger::new(target) {
                Ok(pinger) => pinger,
                Err(error) => {
                    tracing::error!("Failed to create pinger: {error}");
                    tracing::error!(
                        "ICMP ping requires elevated privileges. On macOS, run with sudo. \
                         On Linux, run: sudo setcap cap_net_raw+ep $(which istherenet)"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            loop {
                // Check if config changed
                let current_config = match config.lock() {
                    Ok(guard) => guard.clone(),
                    Err(_) => break,
                };
                if current_config != config_snapshot {
                    break;
                }

                let rtt_ms = match pinger.ping(config_snapshot.ping_timeout()).await {
                    Ok(result) => result,
                    Err(error) => {
                        tracing::error!("Ping error: {error}");
                        None
                    }
                };

                let new_status = debouncer.observe(
                    rtt_ms,
                    config_snapshot.ping_slow_threshold_milliseconds,
                    current_status,
                );

                if let Some(ref status) = new_status {
                    tracing::info!("Internet connection: {status}");

                    let color = color_for_status(status.kind(), &config_snapshot.colors);
                    let fade_after = match status.kind() {
                        NetStatusKind::Connected => config_snapshot.fade_seconds.connected,
                        NetStatusKind::Disconnected => config_snapshot.fade_seconds.disconnected,
                        NetStatusKind::Slow => config_snapshot.fade_seconds.slow,
                    };

                    overlay.show(*color, fade_after);

                    if let Some(ref command) = config_snapshot.shell_command_on_status_change {
                        if !command.is_empty() {
                            run_shell_command(command, status);
                        }
                    }

                    current_status = Some(status.kind());
                }

                tokio::time::sleep(config_snapshot.ping_interval()).await;
            }
        }
    });
}
