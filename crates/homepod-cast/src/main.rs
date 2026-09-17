//! VulpiCast — stream Windows system audio to a HomePod over AirPlay 2.
//!
//! Run with no arguments to launch the system-tray app. Run `--list` to print
//! discovered AirPlay devices and exit.

// Use the Windows subsystem (no console window) for the normal tray app, but
// keep a console when built for debugging via `--list`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cast;
mod settings_window;
mod tray;

use std::time::Duration;

/// Raise the Windows timer resolution to 1ms for the lifetime of the process.
///
/// Without this a process gets the default ~15.6ms scheduler granularity, so
/// *every* short sleep or channel timeout in the audio path is rounded up to a
/// full tick. In the real-time send loop that turns a 2ms wait into a 15.6ms
/// stall, which starves the sender thread and is audible as crackling.
struct TimerResolution;

impl TimerResolution {
    fn acquire() -> Self {
        let r = unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) };
        if r != 0 {
            tracing::warn!("could not raise timer resolution to 1ms (code {r})");
        }
        Self
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Media::timeEndPeriod(1) };
    }
}

fn main() -> anyhow::Result<()> {
    let log_path = cast::init_logging()?;
    let _timer_resolution = TimerResolution::acquire();

    let args: Vec<String> = std::env::args().collect();

    // Diagnostic: reproduce start -> stream -> stop -> restart -> stream with logs.
    if args.iter().any(|a| a == "--selftest") {
        println!("VulpiCast self-test; log: {}", log_path.display());
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let devices = match cast::discover(Duration::from_secs(3)).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("discover failed: {e:#}");
                    return;
                }
            };
            let Some(dev) = devices.into_iter().next() else {
                tracing::error!("no device found");
                return;
            };
            tracing::info!("selftest target: {} ({})", dev.name, dev.model);
            tracing::info!("=== starting session ===");
            match cast::Session::start(
                dev.clone(),
                cast::DEFAULT_VOLUME,
                cast::load_mode(),
            )
            .await
            {
                Ok(mut s) => {
                    let secs = 50u32;
                    tracing::info!("streaming for {secs}s with 2s keepalive (play audio now)");
                    let mut elapsed = 0;
                    while elapsed < secs {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if let Err(e) = s.feedback().await {
                            tracing::error!("keepalive failed: {e:#}");
                            break;
                        }
                        elapsed += 2;
                    }
                    s.stop().await;
                    tracing::info!("stopped");
                }
                Err(e) => tracing::error!("start failed: {e:#}"),
            }
            tracing::info!("selftest complete");
        });
        // The library leaks an infinite spawn_blocking task; force shutdown so
        // we don't hang on runtime drop.
        rt.shutdown_timeout(Duration::from_millis(300));
        return Ok(());
    }

    if args.iter().any(|a| a == "--list") {
        let rt = tokio::runtime::Runtime::new()?;
        let devices = rt.block_on(cast::discover(Duration::from_secs(3)))?;
        if devices.is_empty() {
            println!("No AirPlay devices found.");
        } else {
            println!("AirPlay 2 devices (legacy AirPlay 1 is intentionally excluded):");
            for d in &devices {
                let ipv4 = d.addresses.iter().find(|a| a.is_ipv4());
                println!(
                    "  {:<24} {:<18} {:<15} firmware={} os={}",
                    d.name,
                    d.model,
                    ipv4.map(|a| a.to_string()).unwrap_or_default(),
                    d.firmware_version.as_deref().unwrap_or("unknown"),
                    d.os_version.as_deref().unwrap_or("unknown"),
                );
            }
        }
        println!("Log: {}", log_path.display());
        return Ok(());
    }

    tray::run()
}
