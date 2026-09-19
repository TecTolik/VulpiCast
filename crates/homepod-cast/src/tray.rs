//! System-tray UI.
//!
//! The tray (main thread) renders state and turns clicks / the global hotkey
//! into commands. A dedicated control thread owns the tokio runtime and the
//! active streaming sessions, and reports the *true* state back so the icon and
//! check marks stay correct (e.g. if a connection fails).
//!
//! Volume, the streaming mode and the global toggle hotkey are configured in a
//! small native settings window (see [`crate::settings_window`]): the slider
//! applies volume live, and Save reports the hotkey (which the main thread
//! re-registers) along with the streaming mode.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG,
};

use crate::cast;
use crate::settings_window;

const WM_HOTKEY: u32 = 0x0312;
const PM_REMOVE: u32 = 0x0001;
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_NOREPEAT: u32 = 0x4000;
const HOTKEY_ID: i32 = 1;

/// Default global toggle hotkey: Ctrl+Alt+H (used until the user picks one).
const DEFAULT_HOTKEY_MODS: u32 = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
const DEFAULT_HOTKEY_VK: u32 = 0x48; // 'H'

/// Commands sent from the tray to the control thread.
enum Cmd {
    SetTargets(Vec<usize>),
    Rescan,
    SetVolume(f32),
    /// Change the Windows render endpoint captured through WASAPI loopback.
    /// Active sessions are restarted so every target switches atomically from
    /// the tray's point of view.
    SetOutput(Option<String>),
    /// New streaming mode. Applied immediately by reconnecting if a stream is
    /// running, since the buffer size is fixed when the stream starts.
    SetMode(cast::StreamingMode),
    Quit,
}

/// True state reported by the control thread back to the tray.
enum Status {
    Streaming(Vec<usize>),
    Stopped,
    Error { active: Vec<usize>, message: String },
}

enum DeviceUpdate {
    Ready(Vec<String>),
    Failed(String),
}

/// Register the given hotkey as the global toggle (unregistering any prior one).
/// `vk == 0` disables the hotkey.
fn set_hotkey(mods: u32, vk: u32) {
    unsafe {
        UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
        if vk != 0 {
            RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, mods, vk);
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let (dev_tx, dev_rx) = std::sync::mpsc::channel::<DeviceUpdate>();
    let (status_tx, status_rx) = std::sync::mpsc::channel::<Status>();
    // New hotkey chosen in the settings window -> main thread re-registers it.
    let (hk_tx, hk_rx) = std::sync::mpsc::channel::<(u32, u32)>();

    // Control thread: scans for devices, then owns the streaming session.
    let control = std::thread::spawn(move || control_loop(cmd_rx, dev_tx, status_tx));

    // Wait for the initial device scan.
    let initial_update = dev_rx
        .recv_timeout(Duration::from_secs(8))
        .unwrap_or_else(|_| DeviceUpdate::Failed("Device discovery timed out".into()));
    let names = match &initial_update {
        DeviceUpdate::Ready(names) => names.clone(),
        DeviceUpdate::Failed(_) => Vec::new(),
    };
    let audio_outputs = cast::discover_audio_outputs().unwrap_or_else(|e| {
        tracing::error!("Windows audio output discovery failed: {e:#}");
        Vec::new()
    });
    let saved_output = cast::load_output_device();
    let mut selected_output =
        saved_output.filter(|id| audio_outputs.iter().any(|output| output.id == *id));
    let _ = cmd_tx.send(Cmd::SetOutput(selected_output.clone()));

    // --- Build the menu ---
    let menu = Menu::new();

    // Each AirPlay target is an independent toggle, so any subset can stream.
    let off_check = CheckMenuItem::new("\u{25A0}  Stopped", true, true, None);
    menu.append(&off_check)?;
    menu.append(&PredefinedMenuItem::separator())?;

    let mut device_checks: Vec<CheckMenuItem> = Vec::new();
    let mut device_ids: Vec<(MenuId, usize)> = Vec::new();
    let mut no_devices_item: Option<MenuItem> = None;
    replace_device_menu(
        &menu,
        &names,
        &mut device_checks,
        &mut device_ids,
        &mut no_devices_item,
    )?;
    menu.append(&PredefinedMenuItem::separator())?;

    let output_menu = Submenu::new("Windows audio output", true);
    let default_name = audio_outputs
        .iter()
        .find(|output| output.is_default)
        .map(|output| format!("System default — {}", output.name))
        .unwrap_or_else(|| "System default".to_string());
    let default_output_check =
        CheckMenuItem::new(default_name, true, selected_output.is_none(), None);
    let default_output_id = default_output_check.id().clone();
    output_menu.append(&default_output_check)?;
    let mut output_checks = Vec::new();
    let mut output_ids = Vec::new();
    for output in &audio_outputs {
        let item = CheckMenuItem::new(
            &output.name,
            true,
            selected_output.as_deref() == Some(output.id.as_str()),
            None,
        );
        output_ids.push((item.id().clone(), output.id.clone()));
        output_menu.append(&item)?;
        output_checks.push(item);
    }
    menu.append(&output_menu)?;
    menu.append(&PredefinedMenuItem::separator())?;

    let rescan_item = MenuItem::new("Rescan for AirPlay 2 devices", true, None);
    let rescan_id = rescan_item.id().clone();
    menu.append(&rescan_item)?;

    let log_item = MenuItem::new("Open log folder", true, None);
    let log_id = log_item.id().clone();
    menu.append(&log_item)?;

    let status_item = MenuItem::new("Ready — AirPlay 2 only", false, None);
    menu.append(&status_item)?;
    if let DeviceUpdate::Failed(message) = initial_update {
        status_item.set_text("Device discovery failed — see log");
        tracing::error!("initial discovery failed: {message}");
    }
    menu.append(&PredefinedMenuItem::separator())?;

    // Settings: opens a window with a volume slider and a hotkey capture box.
    let settings_item = MenuItem::new("Settings\u{2026}", true, None);
    menu.append(&settings_item)?;
    let settings_id = settings_item.id().clone();
    menu.append(&PredefinedMenuItem::separator())?;

    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&quit_item)?;
    let quit_id = quit_item.id().clone();
    let off_id = off_check.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_tooltip("VulpiCast — ready (AirPlay 2 only)")
        .with_icon(make_icon(false))
        .build()?;

    // Register the saved toggle hotkey (or the default if none saved yet).
    let (mut cur_mods, mut cur_vk) =
        cast::load_hotkey().unwrap_or((DEFAULT_HOTKEY_MODS, DEFAULT_HOTKEY_VK));
    set_hotkey(cur_mods, cur_vk);

    let menu_rx = tray_icon::menu::MenuEvent::receiver();

    // UI state (optimistic; corrected by Status from the control thread).
    let mut active = BTreeSet::<usize>::new();
    let mut last_targets = BTreeSet::<usize>::new();

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    let mut quit = false;
    while !quit {
        // Pump Windows messages (drives the tray + delivers WM_HOTKEY).
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_HOTKEY && msg.wParam == HOTKEY_ID as usize {
                if !active.is_empty() {
                    last_targets = active.clone();
                    active.clear();
                } else if !device_checks.is_empty() {
                    active = last_targets
                        .iter()
                        .copied()
                        .filter(|idx| *idx < device_checks.len())
                        .collect();
                    if active.is_empty() {
                        active.insert(0);
                    }
                }
                let _ = cmd_tx.send(Cmd::SetTargets(active.iter().copied().collect()));
                apply_state(&off_check, &device_checks, &tray, &active);
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Menu clicks.
        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == quit_id {
                let _ = cmd_tx.send(Cmd::Quit);
                quit = true;
            } else if ev.id == off_id {
                if !active.is_empty() {
                    last_targets = active.clone();
                    active.clear();
                    let _ = cmd_tx.send(Cmd::SetTargets(Vec::new()));
                }
                apply_state(&off_check, &device_checks, &tray, &active);
            } else if ev.id == settings_id {
                // Open the settings window: volume applies live via Cmd::SetVolume;
                // on Save the hotkey comes back on hk_tx for the main thread to
                // register and the streaming mode goes straight to the control
                // thread, which owns the session.
                let vol_tx = cmd_tx.clone();
                let mode_tx = cmd_tx.clone();
                let hotkey_tx = hk_tx.clone();
                settings_window::open(
                    cast::load_volume(),
                    cur_mods,
                    cur_vk,
                    cast::load_mode(),
                    move |v| {
                        let _ = vol_tx.send(Cmd::SetVolume(v));
                    },
                    move |m, vk, mode| {
                        let _ = hotkey_tx.send((m, vk));
                        let _ = mode_tx.send(Cmd::SetMode(mode));
                    },
                );
            } else if ev.id == rescan_id {
                let _ = cmd_tx.send(Cmd::Rescan);
                active.clear();
                status_item.set_text("Scanning\u{2026}");
                apply_state(&off_check, &device_checks, &tray, &active);
            } else if ev.id == log_id {
                if let Err(e) = std::process::Command::new("explorer.exe")
                    .arg(cast::app_dir())
                    .spawn()
                {
                    tracing::error!("failed to open log folder: {e}");
                }
            } else if ev.id == default_output_id {
                if selected_output.is_some() {
                    selected_output = None;
                    cast::save_output_device(None);
                    let _ = cmd_tx.send(Cmd::SetOutput(None));
                    apply_output_state(
                        &default_output_check,
                        &output_checks,
                        &audio_outputs,
                        selected_output.as_deref(),
                    );
                    status_item.set_text("Switching Windows audio output\u{2026}");
                }
            } else if let Some(output_id) = output_ids
                .iter()
                .find(|(id, _)| *id == ev.id)
                .map(|(_, id)| id.clone())
            {
                if selected_output.as_deref() != Some(output_id.as_str()) {
                    selected_output = Some(output_id);
                    cast::save_output_device(selected_output.as_deref());
                    let _ = cmd_tx.send(Cmd::SetOutput(selected_output.clone()));
                    apply_output_state(
                        &default_output_check,
                        &output_checks,
                        &audio_outputs,
                        selected_output.as_deref(),
                    );
                    status_item.set_text("Switching Windows audio output\u{2026}");
                }
            } else if let Some(idx) = device_ids
                .iter()
                .find(|(id, _)| *id == ev.id)
                .map(|(_, i)| *i)
            {
                if !active.remove(&idx) {
                    active.insert(idx);
                }
                if !active.is_empty() {
                    last_targets = active.clone();
                }
                let _ = cmd_tx.send(Cmd::SetTargets(active.iter().copied().collect()));
                apply_state(&off_check, &device_checks, &tray, &active);
            }
        }

        while let Ok(update) = dev_rx.try_recv() {
            active.clear();
            let mut tooltip_override = None;
            match update {
                DeviceUpdate::Ready(names) => {
                    replace_device_menu(
                        &menu,
                        &names,
                        &mut device_checks,
                        &mut device_ids,
                        &mut no_devices_item,
                    )?;
                    last_targets.clear();
                    status_item.set_text("Ready — AirPlay 2 only");
                }
                DeviceUpdate::Failed(message) => {
                    tracing::error!("discovery failed: {message}");
                    replace_device_menu(
                        &menu,
                        &[],
                        &mut device_checks,
                        &mut device_ids,
                        &mut no_devices_item,
                    )?;
                    status_item.set_text("Device discovery failed — see log");
                    tooltip_override = Some("VulpiCast — device discovery failed".to_string());
                }
            }
            apply_state(&off_check, &device_checks, &tray, &active);
            if let Some(tooltip) = tooltip_override {
                let _ = tray.set_tooltip(Some(tooltip));
            }
        }

        // Hotkey chosen in the settings window: re-register and persist.
        while let Ok((m, vk)) = hk_rx.try_recv() {
            cur_mods = m;
            cur_vk = vk;
            set_hotkey(cur_mods, cur_vk);
            cast::save_hotkey(cur_mods, cur_vk);
        }

        // True state from the control thread (corrects optimistic guesses).
        while let Ok(st) = status_rx.try_recv() {
            let mut tooltip_override = None;
            active = match st {
                Status::Streaming(indices) => {
                    let active: BTreeSet<_> = indices.into_iter().collect();
                    if !active.is_empty() {
                        last_targets = active.clone();
                    }
                    status_item.set_text(if active.len() == 1 {
                        "Connected to 1 AirPlay 2 device"
                    } else {
                        "Connected to multiple AirPlay 2 devices"
                    });
                    active
                }
                Status::Stopped => {
                    status_item.set_text("Ready — AirPlay 2 only");
                    BTreeSet::new()
                }
                Status::Error {
                    active: indices,
                    message,
                } => {
                    tracing::error!("streaming error: {message}");
                    let active: BTreeSet<_> = indices.into_iter().collect();
                    status_item.set_text(if active.is_empty() {
                        "Connection failed — see log"
                    } else {
                        "Some connections failed — see log"
                    });
                    let short = if message.chars().count() > 100 {
                        format!("{}\u{2026}", message.chars().take(100).collect::<String>())
                    } else {
                        message
                    };
                    tooltip_override = Some(format!("VulpiCast — {short}"));
                    active
                }
            };
            apply_state(&off_check, &device_checks, &tray, &active);
            if let Some(tooltip) = tooltip_override {
                let _ = tray.set_tooltip(Some(tooltip));
            }
        }

        if quit {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    unsafe { UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID) };
    let _ = control.join();
    // Guarantee termination even if a leaked library thread lingers.
    std::process::exit(0);
}

/// Reflect the current state in the check marks and tray icon.
fn apply_state(
    off: &CheckMenuItem,
    devices: &[CheckMenuItem],
    tray: &TrayIcon,
    active: &BTreeSet<usize>,
) {
    off.set_checked(active.is_empty());
    for (i, c) in devices.iter().enumerate() {
        c.set_checked(active.contains(&i));
    }
    let _ = tray.set_icon(Some(make_icon(!active.is_empty())));
    if active.len() == 1 {
        let _ = tray.set_tooltip(Some("VulpiCast — streaming to 1 AirPlay 2 device"));
    } else if active.len() > 1 {
        let _ = tray.set_tooltip(Some(format!(
            "VulpiCast — streaming to {} AirPlay 2 devices",
            active.len()
        )));
    } else {
        let _ = tray.set_tooltip(Some("VulpiCast — ready (AirPlay 2 only)"));
    }
}

fn apply_output_state(
    default_output: &CheckMenuItem,
    outputs: &[CheckMenuItem],
    available: &[cast::AudioOutput],
    selected_id: Option<&str>,
) {
    default_output.set_checked(selected_id.is_none());
    for (item, output) in outputs.iter().zip(available) {
        item.set_checked(selected_id == Some(output.id.as_str()));
    }
}

fn replace_device_menu(
    menu: &Menu,
    names: &[String],
    checks: &mut Vec<CheckMenuItem>,
    ids: &mut Vec<(MenuId, usize)>,
    no_devices: &mut Option<MenuItem>,
) -> anyhow::Result<()> {
    for item in checks.drain(..) {
        let _ = menu.remove(&item);
    }
    ids.clear();
    if let Some(item) = no_devices.take() {
        let _ = menu.remove(&item);
    }

    if names.is_empty() {
        let item = MenuItem::new("No AirPlay 2 devices found", false, None);
        menu.insert(&item, 2)?;
        *no_devices = Some(item);
    } else {
        for (index, name) in names.iter().enumerate() {
            let item = CheckMenuItem::new(format!("\u{25B6}  {name}"), true, false, None);
            ids.push((item.id().clone(), index));
            menu.insert(&item, 2 + index)?;
            checks.push(item);
        }
    }
    Ok(())
}

fn normalize_targets(targets: Vec<usize>, device_count: usize) -> BTreeSet<usize> {
    targets
        .into_iter()
        .filter(|idx| *idx < device_count)
        .collect()
}

fn active_indices(sessions: &BTreeMap<usize, cast::Session>) -> Vec<usize> {
    sessions.keys().copied().collect()
}

fn report_sessions(
    status_tx: &Sender<Status>,
    sessions: &BTreeMap<usize, cast::Session>,
    errors: Vec<String>,
) {
    let active = active_indices(sessions);
    let status = if !errors.is_empty() {
        Status::Error {
            active,
            message: errors.join("; "),
        }
    } else if active.is_empty() {
        Status::Stopped
    } else {
        Status::Streaming(active)
    };
    let _ = status_tx.send(status);
}

fn stop_all_sessions(rt: &tokio::runtime::Runtime, sessions: &mut BTreeMap<usize, cast::Session>) {
    while let Some((idx, session)) = sessions.pop_first() {
        rt.block_on(session.stop());
        tracing::info!(target_index = idx, "stopped AirPlay stream");
    }
}

fn reconcile_targets(
    rt: &tokio::runtime::Runtime,
    devices: &[airplay_core::device::Device],
    sessions: &mut BTreeMap<usize, cast::Session>,
    targets: Vec<usize>,
    volume: f32,
    mode: cast::StreamingMode,
    output_device_id: Option<&str>,
) -> Vec<String> {
    let desired = normalize_targets(targets, devices.len());
    let removed: Vec<_> = sessions
        .keys()
        .copied()
        .filter(|idx| !desired.contains(idx))
        .collect();
    for idx in removed {
        if let Some(session) = sessions.remove(&idx) {
            rt.block_on(session.stop());
            tracing::info!(target_index = idx, "stopped AirPlay stream");
        }
    }

    let mut errors = Vec::new();
    for idx in desired {
        if sessions.contains_key(&idx) {
            continue;
        }
        let device = devices[idx].clone();
        let name = device.name.clone();
        match rt.block_on(cast::Session::start(
            device,
            volume,
            mode,
            output_device_id.map(str::to_owned),
        )) {
            Ok(session) => {
                tracing::info!(device = %name, "streaming started");
                sessions.insert(idx, session);
            }
            Err(e) => {
                tracing::error!(device = %name, "failed to start streaming: {e:#}");
                errors.push(format!("Connection to {name} failed: {e:#}"));
            }
        }
    }
    errors
}

/// Owns the tokio runtime and all active sessions; reports state via `status_tx`.
fn control_loop(cmd_rx: Receiver<Cmd>, dev_tx: Sender<DeviceUpdate>, status_tx: Sender<Status>) {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("failed to start runtime: {e}");
            let _ = dev_tx.send(DeviceUpdate::Failed(e.to_string()));
            return;
        }
    };

    let mut devices = match rt.block_on(cast::discover(Duration::from_secs(3))) {
        Ok(devices) => {
            let _ = dev_tx.send(DeviceUpdate::Ready(
                devices.iter().map(|d| d.name.clone()).collect(),
            ));
            devices
        }
        Err(e) => {
            let _ = dev_tx.send(DeviceUpdate::Failed(format!("{e:#}")));
            Vec::new()
        }
    };

    let mut sessions = BTreeMap::<usize, cast::Session>::new();
    let mut volume = cast::load_volume();
    let mut mode = cast::load_mode();
    let mut output_device_id = cast::load_output_device();
    let mut last_feedback = std::time::Instant::now();
    loop {
        match cmd_rx.recv_timeout(Duration::from_millis(1000)) {
            Ok(Cmd::SetTargets(targets)) => {
                let errors = reconcile_targets(
                    &rt,
                    &devices,
                    &mut sessions,
                    targets,
                    volume,
                    mode,
                    output_device_id.as_deref(),
                );
                last_feedback = std::time::Instant::now();
                report_sessions(&status_tx, &sessions, errors);
            }
            Ok(Cmd::Rescan) => {
                stop_all_sessions(&rt, &mut sessions);
                let _ = status_tx.send(Status::Stopped);
                match rt.block_on(cast::discover(Duration::from_secs(3))) {
                    Ok(found) => {
                        devices = found;
                        let _ = dev_tx.send(DeviceUpdate::Ready(
                            devices.iter().map(|d| d.name.clone()).collect(),
                        ));
                    }
                    Err(e) => {
                        devices.clear();
                        let _ = dev_tx.send(DeviceUpdate::Failed(format!("{e:#}")));
                    }
                }
            }
            Ok(Cmd::SetVolume(v)) => {
                volume = v;
                cast::save_volume(v);
                for session in sessions.values_mut() {
                    rt.block_on(session.set_volume(v));
                }
            }
            Ok(Cmd::SetMode(new_mode)) => {
                if new_mode != mode {
                    mode = new_mode;
                    cast::save_mode(mode);
                    tracing::info!("streaming mode set to {}ms buffer", mode.latency_ms());
                    let targets = active_indices(&sessions);
                    stop_all_sessions(&rt, &mut sessions);
                    let errors = reconcile_targets(
                        &rt,
                        &devices,
                        &mut sessions,
                        targets,
                        volume,
                        mode,
                        output_device_id.as_deref(),
                    );
                    last_feedback = std::time::Instant::now();
                    report_sessions(&status_tx, &sessions, errors);
                }
            }
            Ok(Cmd::SetOutput(new_output_device_id)) => {
                if new_output_device_id != output_device_id {
                    output_device_id = new_output_device_id;
                    let targets = active_indices(&sessions);
                    stop_all_sessions(&rt, &mut sessions);
                    let errors = reconcile_targets(
                        &rt,
                        &devices,
                        &mut sessions,
                        targets,
                        volume,
                        mode,
                        output_device_id.as_deref(),
                    );
                    last_feedback = std::time::Instant::now();
                    report_sessions(&status_tx, &sessions, errors);
                }
            }
            Ok(Cmd::Quit) => {
                stop_all_sessions(&rt, &mut sessions);
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Periodic AirPlay keepalive — without it the HomePod tears down the
        // session and audio stops after a short while.
        if !sessions.is_empty() && last_feedback.elapsed() >= Duration::from_secs(2) {
            last_feedback = std::time::Instant::now();
            let mut failed = Vec::new();
            for (&idx, session) in sessions.iter_mut() {
                if let Err(e) = rt.block_on(session.feedback()) {
                    let name = devices
                        .get(idx)
                        .map(|device| device.name.as_str())
                        .unwrap_or("unknown device");
                    failed.push((idx, format!("{name}: {e:#}")));
                }
            }
            if !failed.is_empty() {
                let mut errors = Vec::new();
                for (idx, message) in failed {
                    tracing::error!(target_index = idx, "AirPlay keepalive failed: {message}");
                    if let Some(session) = sessions.remove(&idx) {
                        rt.block_on(session.stop());
                    }
                    errors.push(format!("AirPlay connection was interrupted: {message}"));
                }
                report_sessions(&status_tx, &sessions, errors);
            }
        }
    }
    // The library leaks an infinite spawn_blocking task per session, so a normal
    // runtime drop would hang. Force shutdown instead.
    rt.shutdown_timeout(Duration::from_millis(300));
}

#[cfg(test)]
mod tests {
    use super::normalize_targets;
    use std::collections::BTreeSet;

    #[test]
    fn target_selection_deduplicates_and_ignores_stale_indices() {
        assert_eq!(
            normalize_targets(vec![2, 0, 2, 7, 1], 3),
            BTreeSet::from([0, 1, 2])
        );
    }

    #[test]
    fn empty_target_selection_stops_all_devices() {
        assert!(normalize_targets(Vec::new(), 3).is_empty());
    }
}

/// Branded 32x32 fox icon; muted while idle and fully colored while streaming.
fn make_icon(active: bool) -> Icon {
    let rgba = if active {
        include_bytes!("../../../assets/vulpicast-tray-active.rgba").as_slice()
    } else {
        include_bytes!("../../../assets/vulpicast-tray-inactive.rgba").as_slice()
    };
    Icon::from_rgba(rgba.to_vec(), 32, 32).expect("valid VulpiCast icon")
}
