//! System-tray UI.
//!
//! The tray (main thread) renders state and turns clicks / the global hotkey
//! into commands. A dedicated control thread owns the tokio runtime and the
//! active streaming session, and reports the *true* state back so the icon and
//! check marks stay correct (e.g. if a connection fails).
//!
//! Volume and the global toggle hotkey are configured in a small native
//! settings window (see [`crate::settings_window`]): the slider applies volume
//! live, and Save reports a new hotkey which the main thread re-registers.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
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
    Start(usize),
    Stop,
    Rescan,
    SetVolume(f32),
    Quit,
}

/// True state reported by the control thread back to the tray.
enum Status {
    Streaming(usize),
    Stopped,
    Error(String),
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
        .unwrap_or_else(|_| DeviceUpdate::Failed("Gerätesuche hat zu lange gedauert".into()));
    let names = match &initial_update {
        DeviceUpdate::Ready(names) => names.clone(),
        DeviceUpdate::Failed(_) => Vec::new(),
    };

    // --- Build the menu ---
    let menu = Menu::new();

    // Single-selection group: "Stopped" + one item per device. Starts stopped.
    let off_check = CheckMenuItem::new("\u{25A0}  Gestoppt", true, true, None);
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

    let rescan_item = MenuItem::new("Neu nach AirPlay-2-Geräten suchen", true, None);
    let rescan_id = rescan_item.id().clone();
    menu.append(&rescan_item)?;

    let log_item = MenuItem::new("Protokollordner öffnen", true, None);
    let log_id = log_item.id().clone();
    menu.append(&log_item)?;

    let status_item = MenuItem::new("Bereit — nur AirPlay 2", false, None);
    menu.append(&status_item)?;
    if let DeviceUpdate::Failed(message) = initial_update {
        status_item.set_text("Gerätesuche fehlgeschlagen — siehe Protokoll");
        tracing::error!("initial discovery failed: {message}");
    }
    menu.append(&PredefinedMenuItem::separator())?;

    // Settings: opens a window with a volume slider and a hotkey capture box.
    let settings_item = MenuItem::new("Einstellungen\u{2026}", true, None);
    menu.append(&settings_item)?;
    let settings_id = settings_item.id().clone();
    menu.append(&PredefinedMenuItem::separator())?;

    let quit_item = MenuItem::new("Beenden", true, None);
    menu.append(&quit_item)?;
    let quit_id = quit_item.id().clone();
    let off_id = off_check.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_tooltip("VulpiCast — bereit (nur AirPlay 2)")
        .with_icon(make_icon(false))
        .build()?;

    // Register the saved toggle hotkey (or the default if none saved yet).
    let (mut cur_mods, mut cur_vk) =
        cast::load_hotkey().unwrap_or((DEFAULT_HOTKEY_MODS, DEFAULT_HOTKEY_VK));
    set_hotkey(cur_mods, cur_vk);

    let menu_rx = tray_icon::menu::MenuEvent::receiver();

    // UI state (optimistic; corrected by Status from the control thread).
    let mut active: Option<usize> = None; // None = stopped
    let mut last_device: usize = 0; // hotkey target when toggling on

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    let mut quit = false;
    while !quit {
        // Pump Windows messages (drives the tray + delivers WM_HOTKEY).
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_HOTKEY && msg.wParam == HOTKEY_ID as usize {
                if active.is_some() {
                    let _ = cmd_tx.send(Cmd::Stop);
                    active = None;
                } else if !device_checks.is_empty() {
                    let _ = cmd_tx.send(Cmd::Start(last_device));
                    active = Some(last_device);
                }
                apply_state(&off_check, &device_checks, &tray, active);
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
                if active.is_some() {
                    let _ = cmd_tx.send(Cmd::Stop);
                    active = None;
                }
                apply_state(&off_check, &device_checks, &tray, active);
            } else if ev.id == settings_id {
                // Open the settings window: volume applies live via Cmd::SetVolume,
                // a chosen hotkey comes back on hk_tx for the main thread to register.
                let vol_tx = cmd_tx.clone();
                let hotkey_tx = hk_tx.clone();
                settings_window::open(
                    cast::load_volume(),
                    cur_mods,
                    cur_vk,
                    move |v| {
                        let _ = vol_tx.send(Cmd::SetVolume(v));
                    },
                    move |m, vk| {
                        let _ = hotkey_tx.send((m, vk));
                    },
                );
            } else if ev.id == rescan_id {
                let _ = cmd_tx.send(Cmd::Rescan);
                active = None;
                status_item.set_text("Suche läuft\u{2026}");
                apply_state(&off_check, &device_checks, &tray, active);
            } else if ev.id == log_id {
                if let Err(e) = std::process::Command::new("explorer.exe")
                    .arg(cast::app_dir())
                    .spawn()
                {
                    tracing::error!("failed to open log folder: {e}");
                }
            } else if let Some(idx) = device_ids
                .iter()
                .find(|(id, _)| *id == ev.id)
                .map(|(_, i)| *i)
            {
                last_device = idx;
                if active != Some(idx) {
                    let _ = cmd_tx.send(Cmd::Start(idx));
                    active = Some(idx);
                }
                apply_state(&off_check, &device_checks, &tray, active);
            }
        }

        while let Ok(update) = dev_rx.try_recv() {
            active = None;
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
                    last_device = 0;
                    status_item.set_text("Bereit — nur AirPlay 2");
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
                    status_item.set_text("Gerätesuche fehlgeschlagen — siehe Protokoll");
                    tooltip_override = Some("VulpiCast — Gerätesuche fehlgeschlagen".to_string());
                }
            }
            apply_state(&off_check, &device_checks, &tray, active);
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
                Status::Streaming(i) => {
                    last_device = i;
                    status_item.set_text("Verbunden über natives AirPlay 2");
                    Some(i)
                }
                Status::Stopped => {
                    status_item.set_text("Bereit — nur AirPlay 2");
                    None
                }
                Status::Error(message) => {
                    tracing::error!("streaming error: {message}");
                    status_item.set_text("Verbindung fehlgeschlagen — siehe Protokoll");
                    let short = if message.chars().count() > 100 {
                        format!("{}\u{2026}", message.chars().take(100).collect::<String>())
                    } else {
                        message
                    };
                    tooltip_override = Some(format!("VulpiCast — {short}"));
                    None
                }
            };
            apply_state(&off_check, &device_checks, &tray, active);
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
    active: Option<usize>,
) {
    off.set_checked(active.is_none());
    for (i, c) in devices.iter().enumerate() {
        c.set_checked(active == Some(i));
    }
    let _ = tray.set_icon(Some(make_icon(active.is_some())));
    if active.is_some() {
        let _ = tray.set_tooltip(Some("VulpiCast — Streaming über AirPlay 2"));
    } else {
        let _ = tray.set_tooltip(Some("VulpiCast — bereit (nur AirPlay 2)"));
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
        let item = MenuItem::new("Keine AirPlay-2-Geräte gefunden", false, None);
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

/// Owns the tokio runtime and the active session; reports state via `status_tx`.
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

    let mut session: Option<cast::Session> = None;
    let mut volume = cast::load_volume();
    let mut last_feedback = std::time::Instant::now();
    loop {
        match cmd_rx.recv_timeout(Duration::from_millis(1000)) {
            Ok(Cmd::Start(idx)) => {
                if let Some(s) = session.take() {
                    rt.block_on(s.stop());
                }
                if let Some(dev) = devices.get(idx).cloned() {
                    let name = dev.name.clone();
                    match rt.block_on(cast::Session::start(dev, volume)) {
                        Ok(s) => {
                            tracing::info!("streaming to {name}");
                            session = Some(s);
                            last_feedback = std::time::Instant::now();
                            let _ = status_tx.send(Status::Streaming(idx));
                        }
                        Err(e) => {
                            tracing::error!("failed to start streaming to {name}: {e:#}");
                            let _ = status_tx.send(Status::Error(format!(
                                "Verbindung zu {name} fehlgeschlagen: {e:#}"
                            )));
                        }
                    }
                }
            }
            Ok(Cmd::Stop) => {
                if let Some(s) = session.take() {
                    rt.block_on(s.stop());
                    tracing::info!("stopped");
                }
                let _ = status_tx.send(Status::Stopped);
            }
            Ok(Cmd::Rescan) => {
                if let Some(s) = session.take() {
                    rt.block_on(s.stop());
                }
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
                if let Some(s) = session.as_mut() {
                    rt.block_on(s.set_volume(v));
                }
            }
            Ok(Cmd::Quit) => {
                if let Some(s) = session.take() {
                    rt.block_on(s.stop());
                }
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Periodic AirPlay keepalive — without it the HomePod tears down the
        // session and audio stops after a short while.
        let feedback_error = if let Some(s) = session.as_mut() {
            if last_feedback.elapsed() >= Duration::from_secs(2) {
                last_feedback = std::time::Instant::now();
                rt.block_on(s.feedback()).err()
            } else {
                None
            }
        } else {
            None
        };
        if let Some(e) = feedback_error {
            tracing::error!("AirPlay keepalive failed: {e:#}");
            if let Some(s) = session.take() {
                rt.block_on(s.stop());
            }
            let _ = status_tx.send(Status::Error(format!(
                "AirPlay-Verbindung wurde unterbrochen: {e:#}"
            )));
        }
    }
    // The library leaks an infinite spawn_blocking task per session, so a normal
    // runtime drop would hang. Force shutdown instead.
    rt.shutdown_timeout(Duration::from_millis(300));
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
