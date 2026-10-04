//! Portable IME control surface over the session bus.
//!
//! Engines register [`Control1`] under a per-binary well-known name
//! (`com.{command}.InputMethod1`). Panel/tray UIs discover the active IME and
//! render menu rows / call `Activate`.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use zbus::interface;

/// D-Bus interface name shared by every engine and panel client.
pub const INTERFACE: &str = "org.inputmethod.Control1";

/// Well-known menu action ids (engines may add their own).
pub mod action {
    pub const TOGGLE_MODE: &str = "toggle_mode";
    pub const HALF_FULL: &str = "half_full";
    pub const SETTINGS: &str = "settings";
    pub const TOGGLE_CHARSET: &str = "toggle_charset";
}

/// One row in the applet dropdown.
///
/// On the bus this is exposed as `(ssbb)` = id, label, enabled, checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            checked: false,
        }
    }

    pub fn as_tuple(&self) -> (String, String, bool, bool) {
        (
            self.id.clone(),
            self.label.clone(),
            self.enabled,
            self.checked,
        )
    }

    pub fn from_tuple(t: (String, String, bool, bool)) -> Self {
        Self {
            id: t.0,
            label: t.1,
            enabled: t.2,
            checked: t.3,
        }
    }
}

/// Common CN/EN + half/full + settings menu (pinyin-style).
pub fn menu_cn_en_half_full_settings(english: bool) -> Vec<MenuItem> {
    let toggle = if english {
        MenuItem::new(action::TOGGLE_MODE, "Switch to Chinese")
    } else {
        MenuItem::new(action::TOGGLE_MODE, "Switch to English")
    };
    vec![
        toggle,
        MenuItem::new(action::HALF_FULL, "Half / Full Width"),
        MenuItem::new(action::SETTINGS, "Settings…"),
    ]
}

/// CN/EN + settings only (no half/full row).
pub fn menu_cn_en_settings(english: bool) -> Vec<MenuItem> {
    let toggle = if english {
        MenuItem::new(action::TOGGLE_MODE, "Switch to Chinese")
    } else {
        MenuItem::new(action::TOGGLE_MODE, "Switch to English")
    };
    vec![
        toggle,
        MenuItem::new(action::SETTINGS, "Settings…"),
    ]
}

/// Events forwarded from D-Bus methods into the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    /// Menu / client activated an action id (`toggle_mode`, `half_full`, …).
    Activate(String),
    SetPassthrough(bool),
}

/// Bus name + object path for one engine binary.
#[derive(Debug, Clone, Copy)]
pub struct BusIdentity {
    pub well_known_name: &'static str,
    pub object_path: &'static str,
}

static MODE_TX: OnceLock<watch::Sender<String>> = OnceLock::new();
static MENU_TX: OnceLock<watch::Sender<Vec<MenuItem>>> = OnceLock::new();
static SETTINGS_CMD_TX: OnceLock<watch::Sender<String>> = OnceLock::new();

/// Publish the panel button label (`中` / `英` / `全` / …).
pub fn publish_mode_label(label: impl Into<String>) {
    if let Some(tx) = MODE_TX.get() {
        let _ = tx.send(label.into());
    }
}

/// Publish the dropdown rows for the generic applet.
pub fn publish_menu(items: Vec<MenuItem>) {
    if let Some(tx) = MENU_TX.get() {
        let _ = tx.send(items);
    }
}

/// Optional settings launch command (e.g. `pinyinwl --settings`).
pub fn publish_settings_command(cmd: impl Into<String>) {
    if let Some(tx) = SETTINGS_CMD_TX.get() {
        let _ = tx.send(cmd.into());
    }
}

struct Control1 {
    tx: mpsc::UnboundedSender<ControlEvent>,
    mode: watch::Receiver<String>,
    menu: watch::Receiver<Vec<MenuItem>>,
    settings_command: watch::Receiver<String>,
}

#[interface(name = "org.inputmethod.Control1")]
impl Control1 {
    #[zbus(property)]
    async fn mode_label(&self) -> String {
        self.mode.borrow().clone()
    }

    /// `a(ssbb)` — id, label, enabled, checked.
    #[zbus(property)]
    async fn menu_items(&self) -> Vec<(String, String, bool, bool)> {
        self.menu.borrow().iter().map(MenuItem::as_tuple).collect()
    }

    #[zbus(property)]
    async fn settings_command(&self) -> String {
        self.settings_command.borrow().clone()
    }

    async fn activate(&self, id: &str) {
        let _ = self.tx.send(ControlEvent::Activate(id.to_string()));
    }

    async fn set_passthrough(&self, passthrough: bool) {
        let _ = self.tx.send(ControlEvent::SetPassthrough(passthrough));
    }

    async fn toggle_mode(&self) {
        let _ = self
            .tx
            .send(ControlEvent::Activate(action::TOGGLE_MODE.to_string()));
    }

    async fn toggle_half_full_width(&self) {
        let _ = self
            .tx
            .send(ControlEvent::Activate(action::HALF_FULL.to_string()));
    }

    async fn open_settings(&self) {
        let _ = self
            .tx
            .send(ControlEvent::Activate(action::SETTINGS.to_string()));
    }
}

/// Register Control1 and return a receiver of [`ControlEvent`]s.
pub async fn serve(
    identity: BusIdentity,
    initial_label: impl Into<String>,
    initial_menu: Vec<MenuItem>,
    settings_command: impl Into<String>,
) -> mpsc::UnboundedReceiver<ControlEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let (mode_tx, mode_rx) = watch::channel(initial_label.into());
    let (menu_tx, menu_rx) = watch::channel(initial_menu);
    let (settings_tx, settings_rx) = watch::channel(settings_command.into());

    let _ = MODE_TX.set(mode_tx.clone());
    let _ = MENU_TX.set(menu_tx.clone());
    let _ = SETTINGS_CMD_TX.set(settings_tx.clone());

    let dbus_obj = Control1 {
        tx,
        mode: mode_rx,
        menu: menu_rx,
        settings_command: settings_rx,
    };

    tokio::spawn(async move {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                log::error!("ime-control: session bus connect failed: {e}");
                return;
            }
        };

        if let Err(e) = conn
            .object_server()
            .at(identity.object_path, dbus_obj)
            .await
        {
            log::error!("ime-control: register object failed: {e}");
            return;
        }

        if let Err(e) = conn.request_name(identity.well_known_name).await {
            log::error!("ime-control: request name failed: {e}");
            return;
        }

        log::info!(
            "ime-control: registered {} ({})",
            identity.well_known_name,
            INTERFACE
        );

        let iface = match conn
            .object_server()
            .interface::<_, Control1>(identity.object_path)
            .await
        {
            Ok(i) => i,
            Err(e) => {
                log::error!("ime-control: interface ref failed: {e}");
                return;
            }
        };

        let mut mode_watch = mode_tx.subscribe();
        let mut menu_watch = menu_tx.subscribe();
        let mut settings_watch = settings_tx.subscribe();
        loop {
            tokio::select! {
                changed = mode_watch.changed() => {
                    if changed.is_err() { break; }
                    let _ = mode_watch.borrow_and_update();
                    let guard = iface.get().await;
                    if let Err(e) = guard.mode_label_changed(iface.signal_emitter()).await {
                        log::warn!("ime-control: ModeLabel signal failed: {e}");
                    }
                }
                changed = menu_watch.changed() => {
                    if changed.is_err() { break; }
                    let _ = menu_watch.borrow_and_update();
                    let guard = iface.get().await;
                    if let Err(e) = guard.menu_items_changed(iface.signal_emitter()).await {
                        log::warn!("ime-control: MenuItems signal failed: {e}");
                    }
                }
                changed = settings_watch.changed() => {
                    if changed.is_err() { break; }
                    let _ = settings_watch.borrow_and_update();
                    let guard = iface.get().await;
                    if let Err(e) = guard.settings_command_changed(iface.signal_emitter()).await {
                        log::warn!("ime-control: SettingsCommand signal failed: {e}");
                    }
                }
            }
        }
    });

    rx
}

/// Derive the D-Bus id segment from an IME command or app_id.
///
/// Accepts `chewingwl`, `/usr/bin/chewingwl`, etc. → `chewingwl`.
pub fn bus_id_from(command_or_app_id: &str) -> String {
    std::path::Path::new(command_or_app_id)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(command_or_app_id)
        .to_string()
}

/// Well-known name: `com.{id}.InputMethod1`.
pub fn bus_name_for_command(command_or_app_id: &str) -> String {
    format!("com.{}.InputMethod1", bus_id_from(command_or_app_id))
}

pub fn object_path_for_command(command_or_app_id: &str) -> String {
    format!("/com/{}/InputMethod1", bus_id_from(command_or_app_id))
}
