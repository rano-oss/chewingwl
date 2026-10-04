mod dbus;
mod settings;
mod state;
mod view;

use chewing::editor::{
    keyboard::{self, AnyKeyboardLayout, KeyboardLayout, Modifiers as Mods, Qwerty},
    BasicEditor,
};

use cosmic::app::Core;
use cosmic::iced::core::event::wayland::input_method::{
    InputMethodEvent, InputMethodKeyboardEvent, KeyEvent, Modifiers,
};
use cosmic::iced::event::{self, listen_raw};
use cosmic::iced::keyboard::key::Named;
use cosmic::iced::keyboard::Key;
use cosmic::iced::platform_specific::runtime::wayland::input_method::InputMethodPopupSettings;
use cosmic::iced::platform_specific::shell::wayland::commands::input_method::{self, PopupPositionMode};
use cosmic::iced::{self, window, Subscription, Task};
use cosmic::iced::{Color, Event, Size};
use cosmic::widget;
use state::{InputMethodState, MAX_VISIBLE_PAGES};

use std::cmp::min;

type CosmicAction = cosmic::Action<Message>;

fn wrap(task: Task<Message>) -> Task<CosmicAction> {
    task.map(cosmic::action::app)
}

fn main() -> iced::Result {
    env_logger::init();
    if std::env::args().any(|a| a == "--settings") {
        return settings::run_settings();
    }
    cosmic::app::run::<ChewingWl>(
        cosmic::app::Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        (),
    )
}

struct ChewingWl {
    core: Core,
    state: InputMethodState,
    settings_window: Option<window::Id>,
    settings_config: settings::ChewingConfig,
    settings_status: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Activate,
    Deactivate,
    KeyPressed(KeyEvent, Key, Modifiers, u32),
    KeyReleased(KeyEvent, Key, Modifiers, u32),
    Modifiers(Modifiers),
    Done,
    DbusToggleMode,
    DbusSetPassthrough(bool),
    DbusToggleHalfFullWidth,
    DbusOpenSettings,
    Settings(settings::Msg),
    SettingsWindowClosed(window::Id),
    SettingsOpened,
    ConfigChanged,
}

impl cosmic::Application for ChewingWl {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.chewingwl.InputMethod";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<CosmicAction>) {
        let mut state = InputMethodState {
            index: 0,
            page: 0,
            editor: state::create_editor(),
            keyboard: AnyKeyboardLayout::Qwerty(Qwerty),
            shift_set: false,
            passthrough_mode: false,
            visible_page_offset: 0,
            num_visible_pages: 1,
            popup_id: window::Id::NONE,
            config: settings::ChewingConfig::default(),
        };

        let config = settings::ChewingConfig::load();
        state.apply_config(&config);
        let start_english = state.config.default_use_english_mode;
        set_english_mode(&mut state, start_english);
        // Fresh session with default English: start in passthrough 英 (no preedit).
        if start_english {
            state.passthrough_mode = true;
        }
        state.publish_mode_status();

        let popup_settings = InputMethodPopupSettings::default();
        state.popup_id = popup_settings.id;
        let task = Task::batch([
            input_method::get_input_method_popup(popup_settings),
            input_method::set_popup_position_mode(PopupPositionMode::FollowCursor),
        ]);
        (
            ChewingWl {
                core,
                state,
                settings_window: None,
                settings_config: config,
                settings_status: None,
            },
            wrap(task),
        )
    }

    fn view(&self) -> cosmic::Element<'_, Message> {
        widget::Space::new().width(0).height(0).into()
    }

    fn view_window(&self, id: window::Id) -> cosmic::Element<'_, Message> {
        if self.settings_window == Some(id) {
            return settings::settings_view(
                &self.settings_config,
                self.settings_status.as_deref(),
            )
            .map(Message::Settings);
        }
        view::view(&self.state, id)
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::SettingsWindowClosed(id))
    }

    fn update(&mut self, message: Message) -> Task<CosmicAction> {
        let state = &mut self.state;
        match message {
            Message::Activate => {
                state.editor.clear();
                state.index = 0;
                state.publish_mode_status();
                wrap(input_method::set_popup_position_mode(PopupPositionMode::FollowCursor))
            }
            Message::Deactivate => {
                let task = apply_switch_im_behavior(state);
                state.index = 0;
                state.reset_selection_view();
                wrap(task)
            }
            Message::KeyPressed(key_event, key, modifiers, serial) => {
                let cmd = handle_key_press(state, key_event, key, modifiers, serial);
                if serial == 0 {
                    // Client-side repeat: no compositor event to filter
                    wrap(cmd.unwrap_or_else(Task::none))
                } else if let Some(cmd) = cmd {
                    wrap(Task::batch([cmd, input_method::filter_key(serial, true)]))
                } else {
                    wrap(input_method::filter_key(serial, false))
                }
            }
            Message::KeyReleased(key_event, key, _modifiers, serial) => {
                // 中/英 toggle must work while composing (in-IME English), not only
                // when the buffer is empty.
                let filter = if is_chi_eng_toggle_key(state, &key, &key_event) && state.shift_set {
                    state.shift_set = false;
                    set_english_mode(state, !state.is_english_mode());
                    true
                } else {
                    state.editor.is_selecting() || !state.preedit().is_empty()
                };
                wrap(input_method::filter_key(serial, filter))
            }
            Message::Modifiers(modifiers) => {
                if state.config.sync_caps_lock == "Keyboard" {
                    set_english_mode(state, modifiers.caps_lock);
                }
                Task::none()
            }
            Message::Done => {
                if state.preedit().is_empty() {
                    wrap(Task::none())
                } else {
                    wrap(state.sync_preedit())
                }
            }
            Message::DbusToggleMode => {
                set_english_mode(state, !state.is_english_mode());
                Task::none()
            }
            Message::DbusSetPassthrough(english) => {
                set_english_mode(state, english);
                Task::none()
            }
            Message::DbusToggleHalfFullWidth => {
                state.toggle_fullwidth();
                Task::none()
            }
            Message::DbusOpenSettings => {
                if let Some(id) = self.settings_window {
                    return wrap(window::gain_focus(id));
                }
                self.settings_config = settings::ChewingConfig::load();
                self.settings_status = None;
                let (id, open) = window::open(window::Settings {
                    size: Size::new(600.0, 800.0),
                    exit_on_close_request: true,
                    decorations: true,
                    transparent: false,
                    ..Default::default()
                });
                self.settings_window = Some(id);
                // Avoid ConfigChanged remount flicker on first open.
                wrap(open.map(|_| Message::SettingsOpened))
            }
            Message::Settings(msg) => {
                self.settings_status = settings::apply_msg(&mut self.settings_config, msg);
                self.state.apply_config(&self.settings_config);
                self.state.publish_mode_status();
                Task::none()
            }
            Message::SettingsOpened => Task::none(),
            Message::SettingsWindowClosed(id) => {
                if self.settings_window == Some(id) {
                    self.settings_window = None;
                    self.settings_status = None;
                }
                wrap(input_method::reset_popup_size())
            }
            Message::ConfigChanged => {
                let config = settings::ChewingConfig::load();
                self.settings_config = config.clone();
                state.apply_config(&config);
                state.publish_mode_status();
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let wayland_sub = listen_raw(|event, status, _id| match (event.clone(), status) {
            (
                Event::PlatformSpecific(event::PlatformSpecific::Wayland(
                    event::wayland::Event::InputMethod(event),
                )),
                event::Status::Ignored,
            ) => match event {
                InputMethodEvent::Activate => Some(Message::Activate),
                InputMethodEvent::Deactivate => Some(Message::Deactivate),
                InputMethodEvent::Done => Some(Message::Done),
                _ => None,
            },
            (
                Event::PlatformSpecific(event::PlatformSpecific::Wayland(
                    event::wayland::Event::InputMethodKeyboard(event),
                )),
                event::Status::Ignored,
            ) => match event {
                InputMethodKeyboardEvent::Press(key, key_code, modifiers, serial) => {
                    Some(Message::KeyPressed(key, key_code, modifiers, serial))
                }
                InputMethodKeyboardEvent::Release(key, key_code, modifiers, serial) => {
                    Some(Message::KeyReleased(key, key_code, modifiers, serial))
                }
                InputMethodKeyboardEvent::Repeat(key, key_code, modifiers, serial) => {
                    Some(Message::KeyPressed(key, key_code, modifiers, serial))
                }
                InputMethodKeyboardEvent::Modifiers(modifiers) => {
                    Some(Message::Modifiers(modifiers))
                }
            },
            _ => None,
        });

        Subscription::batch([
            wayland_sub,
            Subscription::run(dbus::dbus_subscription),
            Subscription::run(config_watcher),
        ])
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // Must stay transparent for every window: iced has a single clear color,
        // and an opaque clear paints the IM popup surface as a solid box that
        // follows the cursor. Settings paints its own opaque fill in-view.
        let cosmic = cosmic::theme::active();
        let cosmic = cosmic.cosmic();
        Some(cosmic::iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: cosmic.on_bg_color().into(),
            icon_color: cosmic.on_bg_color().into(),
        })
    }
}

/// Handle a key press. Returns Some(task) if the key was consumed, None if passed through.
fn handle_key_press(
    state: &mut InputMethodState,
    key_event: KeyEvent,
    key: Key,
    modifiers: Modifiers,
    _serial: u32,
) -> Option<Task<Message>> {
    // Arm 中/英 toggle on press in every mode (including active preedit).
    if is_chi_eng_toggle_key(state, &key, &key_event) {
        state.shift_set = true;
        return Some(Task::none());
    }
    // Shift+letter (or any other key) must not flip mode on Shift release.
    state.shift_set = false;

    // Ctrl+1 opens symbols; Ctrl+2…9 learn user phrases (ibus Chewing UX).
    if modifiers.ctrl {
        if let Some(code) = ctrl_digit_keycode(&key, &key_event) {
            state.editor.process_keyevent(
                state
                    .keyboard
                    .map_with_mod(code, Mods::control()),
            );
            return Some(state.process_key_and_sync());
        }
    }
    if state.editor.is_selecting() {
        return Some(handle_selecting(state, &key, &key_event, modifiers));
    }
    if !state.preedit().is_empty() {
        return Some(handle_entering(state, key_event, key, modifiers));
    }
    handle_normal(state, key_event, key, modifiers)
}

fn handle_selecting(
    state: &mut InputMethodState,
    key: &Key,
    key_event: &KeyEvent,
    _modifiers: Modifiers,
) -> Task<Message> {
    if let Some(idx) = selection_index(state, key, key_event) {
        state.index = idx;
        return state.select_candidate();
    }

    let arrow_select = state.config.select_candidate_with_arrow_key;
    match key.as_ref() {
        Key::Named(Named::ArrowDown) => {
            if arrow_select && move_selection_index(state, 1) {
                Task::none()
            } else if arrow_select {
                // Past last candidate page: hand off to chewing (interval / next set).
                state.send_key(keyboard::KeyCode::Down);
                state.reset_selection_view();
                state.sync_preedit()
            } else {
                let page_len = page_len(state);
                if state.index + 1 < page_len {
                    state.index += 1;
                    Task::none()
                } else {
                    state.send_key(keyboard::KeyCode::Down);
                    state.reset_selection_view();
                    state.sync_preedit()
                }
            }
        }
        Key::Named(Named::ArrowUp) => {
            if arrow_select && move_selection_index(state, -1) {
                Task::none()
            } else if arrow_select {
                state.send_key(keyboard::KeyCode::Up);
                state.reset_selection_view();
                state.sync_preedit()
            } else {
                state.index = state.index.saturating_sub(1);
                Task::none()
            }
        }
        Key::Named(Named::ArrowLeft) => {
            if arrow_select {
                if !move_selection_index(state, -1) {
                    state.send_key(keyboard::KeyCode::Left);
                    state.reset_selection_view();
                    return state.sync_preedit();
                }
            } else {
                turn_page(state, -1);
            }
            Task::none()
        }
        Key::Named(Named::ArrowRight) => {
            if arrow_select {
                if !move_selection_index(state, 1) {
                    state.send_key(keyboard::KeyCode::Right);
                    state.reset_selection_view();
                    return state.sync_preedit();
                }
            } else {
                turn_page(state, 1);
            }
            Task::none()
        }
        Key::Named(Named::PageUp) => {
            turn_page(state, -1);
            Task::none()
        }
        Key::Named(Named::PageDown) => {
            turn_page(state, 1);
            Task::none()
        }
        Key::Named(Named::Enter) => state.select_candidate(),
        Key::Character(" ") if arrow_select => state.select_candidate(),
        Key::Named(Named::Escape) => {
            let _ = state.editor.cancel_selecting();
            state.index = 0;
            state.sync_preedit()
        }
        _ => Task::none(),
    }
}

fn move_selection_index(state: &mut InputMethodState, delta: isize) -> bool {
    let len = page_len(state) as isize;
    if len == 0 {
        return false;
    }
    let next = state.index as isize + delta;
    if next < 0 {
        if state.page == 0 {
            // Past first page upward — let libchewing handle (interval / exit).
            return false;
        }
        turn_page(state, -1);
        state.index = page_len(state).saturating_sub(1);
        true
    } else if next >= len {
        if state.page + 1 < state.total_pages() {
            turn_page(state, 1);
            state.index = 0;
            clamp_index_to_page(state);
            true
        } else {
            // Past last page downward — libchewing advances interval / candidates.
            false
        }
    } else {
        state.index = next as usize;
        true
    }
}

fn selection_index(
    state: &InputMethodState,
    key: &Key,
    key_event: &KeyEvent,
) -> Option<usize> {
    let keys = &state.config.selection_keys;
    if let Key::Character(c) = key.as_ref() {
        if c.len() == 1 && keys.contains(c) {
            return keys.find(c);
        }
    }
    if state.config.use_keypad_as_selection_key {
        let digit = keypad_digit(key_event)?;
        let ch = if digit == 0 {
            '0'
        } else {
            char::from(b'0' + digit)
        };
        return keys.find(ch);
    }
    None
}

fn keypad_digit(key_event: &KeyEvent) -> Option<u8> {
    // XKB keysyms KP_0…KP_9
    const KP_0: u32 = 0xffb0;
    const KP_9: u32 = 0xffb9;
    if (KP_0..=KP_9).contains(&key_event.keysym) {
        Some((key_event.keysym - KP_0) as u8)
    } else {
        None
    }
}

fn ctrl_digit_keycode(key: &Key, key_event: &KeyEvent) -> Option<keyboard::KeyCode> {
    if let Key::Character(c) = key.as_ref() {
        return match c {
            "1" => Some(keyboard::KeyCode::N1),
            "2" => Some(keyboard::KeyCode::N2),
            "3" => Some(keyboard::KeyCode::N3),
            "4" => Some(keyboard::KeyCode::N4),
            "5" => Some(keyboard::KeyCode::N5),
            "6" => Some(keyboard::KeyCode::N6),
            "7" => Some(keyboard::KeyCode::N7),
            "8" => Some(keyboard::KeyCode::N8),
            "9" => Some(keyboard::KeyCode::N9),
            "0" => Some(keyboard::KeyCode::N0),
            _ => None,
        };
    }
    // keysym Digit1–0
    match key_event.keysym {
        0x0031 => Some(keyboard::KeyCode::N1),
        0x0032 => Some(keyboard::KeyCode::N2),
        0x0033 => Some(keyboard::KeyCode::N3),
        0x0034 => Some(keyboard::KeyCode::N4),
        0x0035 => Some(keyboard::KeyCode::N5),
        0x0036 => Some(keyboard::KeyCode::N6),
        0x0037 => Some(keyboard::KeyCode::N7),
        0x0038 => Some(keyboard::KeyCode::N8),
        0x0039 => Some(keyboard::KeyCode::N9),
        0x0030 => Some(keyboard::KeyCode::N0),
        _ => None,
    }
}

fn page_len(state: &InputMethodState) -> usize {
    let all = state.editor.all_candidates().unwrap_or_default();
    let start = state.page * state.config.candidates_per_page;
    let end = min(start + state.config.candidates_per_page, all.len());
    end - start
}

fn clamp_index_to_page(state: &mut InputMethodState) {
    let len = page_len(state);
    state.index = min(state.index, len.saturating_sub(1));
}

fn turn_page(state: &mut InputMethodState, delta: isize) {
    if delta < 0 {
        if state.page == 0 {
            return;
        }
        state.page -= 1;
        state.num_visible_pages = MAX_VISIBLE_PAGES;
        if state.page < state.visible_page_offset {
            state.visible_page_offset = state.page.saturating_sub(state.num_visible_pages - 1);
        }
    } else if state.page + 1 < state.total_pages() {
        state.page += 1;
        state.num_visible_pages = MAX_VISIBLE_PAGES;
        if state.page >= state.visible_page_offset + state.num_visible_pages {
            state.visible_page_offset = state.page;
        }
    } else {
        return;
    }
    clamp_index_to_page(state);
}

fn process_ascii_char(state: &mut InputMethodState, key_event: &KeyEvent) -> Option<Task<Message>> {
    let ch = key_event.utf8.as_ref().and_then(|s| s.chars().last())?;
    let ch = if state.is_english_mode() {
        apply_english_case(&state.config, ch)
    } else {
        ch
    };
    state
        .editor
        .process_keyevent(state.keyboard.map_ascii(ch as u8));
    Some(state.process_key_and_sync())
}

fn handle_entering(
    state: &mut InputMethodState,
    key_event: KeyEvent,
    key: Key,
    modifiers: Modifiers,
) -> Task<Message> {
    match key.as_ref() {
        Key::Named(Named::Backspace) => state.send_key_and_sync(keyboard::KeyCode::Backspace),
        Key::Character(" ") => {
            if modifiers.shift {
                state.process_shift_space();
            } else {
                state.send_key(keyboard::KeyCode::Space);
            }
            state.process_key_and_sync()
        }
        Key::Named(Named::Enter) => {
            let _ = state.editor.commit();
            state.process_key_and_sync()
        }
        Key::Named(Named::Escape) => {
            state.editor.clear();
            state.maybe_enter_english_passthrough();
            state.publish_mode_status();
            state.sync_preedit()
        }
        Key::Named(Named::Delete) => state.send_key_and_sync(keyboard::KeyCode::Del),
        Key::Named(Named::ArrowLeft) => state.send_key_and_preedit(keyboard::KeyCode::Left),
        Key::Named(Named::ArrowRight) => state.send_key_and_preedit(keyboard::KeyCode::Right),
        Key::Named(Named::ArrowDown) => {
            state.send_key(keyboard::KeyCode::Down);
            if state.editor.is_selecting() {
                state.reset_selection_view();
            }
            state.sync_preedit()
        }
        Key::Named(Named::ArrowUp) => state.send_key_and_preedit(keyboard::KeyCode::Up),
        Key::Named(Named::Tab) => state.send_key_and_sync(keyboard::KeyCode::Tab),
        _ => process_ascii_char(state, &key_event).unwrap_or_else(Task::none),
    }
}

/// Switch chewing LanguageMode (中/英).
///
/// - Entering 英 with an active preedit keeps keys in the editor so Latin can be
///   mixed into the composition.
/// - Entering 英 with an empty buffer uses passthrough (direct typing).
/// - Leaving 英 always clears passthrough so 中 continues producing preedit.
fn set_english_mode(state: &mut InputMethodState, english: bool) {
    use chewing::editor::LanguageMode;

    let want = if english {
        LanguageMode::English
    } else {
        LanguageMode::Chinese
    };
    let mut opts = state.editor.editor_options();
    let mode_changed = opts.language_mode != want;
    if mode_changed {
        opts.language_mode = want;
        state.editor.set_editor_options(opts);
    }

    let prev_passthrough = state.passthrough_mode;
    // Invariant: passthrough only when English AND preedit empty.
    // Non-empty preedit + 英 => in-IME English (keys update composition).
    if english {
        // Fullwidth Latin must go through chewing; passthrough skips conversion.
        state.passthrough_mode = state.preedit().is_empty() && !state.is_fullwidth();
    } else {
        state.passthrough_mode = false;
    }

    if mode_changed || prev_passthrough != state.passthrough_mode {
        state.publish_mode_status();
        if mode_changed && state.config.notify_mode_change {
            let label = state.mode_status_text().to_string();
            tokio::spawn(async move {
                if let Err(e) = notify_mode_change(&label).await {
                    log::warn!("Failed to send mode notification: {e}");
                }
            });
        }
    }
}

pub(crate) async fn notify_mode_change(body: &str) -> zbus::Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    let conn = zbus::Connection::session().await?;
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "chewingwl",
            0u32,
            "",
            "Input mode",
            body,
            Vec::<String>::new(),
            HashMap::<String, Value<'_>>::new(),
            2000i32,
        ),
    )
    .await?;
    Ok(())
}

fn apply_switch_im_behavior(state: &mut InputMethodState) -> Task<Message> {
    match state.config.switch_im_behavior.as_str() {
        "Keep" => Task::none(),
        "CommitPreedit" => {
            let text = state.preedit();
            state.editor.clear();
            state.passthrough_mode = false;
            if text.is_empty() {
                Task::none()
            } else {
                state.send_commit(text)
            }
        }
        "CommitDefault" => {
            if state.editor.is_selecting() {
                state.index = 0;
                return state.select_candidate();
            }
            let text = state.preedit();
            if text.is_empty() {
                state.editor.clear();
                return Task::none();
            }
            let _ = state.editor.commit();
            state.process_key_and_sync()
        }
        // "Clear" and unknown
        _ => {
            state.editor.clear();
            state.passthrough_mode = false;
            Task::batch([
                input_method::reset_popup_size(),
                input_method::set_preedit_string(String::new(), 0, 0),
                input_method::commit(),
            ])
        }
    }
}

/// XKB keysyms for left/right Shift (libxkbcommon).
const KEYSYM_SHIFT_L: u32 = 0xffe1;
const KEYSYM_SHIFT_R: u32 = 0xffe2;

/// Check if the given key matches the configured Chinese/English mode toggle key
fn is_chi_eng_toggle_key(state: &InputMethodState, key: &Key, key_event: &KeyEvent) -> bool {
    match state.config.chi_eng_mode_toggle.as_str() {
        "Disable" => false,
        "CapsLock" => key == &Key::Named(Named::CapsLock),
        "ShiftL" => key == &Key::Named(Named::Shift) && key_event.keysym == KEYSYM_SHIFT_L,
        "ShiftR" => key == &Key::Named(Named::Shift) && key_event.keysym == KEYSYM_SHIFT_R,
        // "Shift" and unknown values: any Shift
        _ => key == &Key::Named(Named::Shift),
    }
}

fn apply_english_case(config: &settings::ChewingConfig, ch: char) -> char {
    match config.default_english_case.as_str() {
        "Lowercase" => ch.to_ascii_lowercase(),
        "Uppercase" => ch.to_ascii_uppercase(),
        _ => ch,
    }
}

/// Handle keys in normal mode (no preedit). Returns Some if consumed.
fn handle_normal(
    state: &mut InputMethodState,
    key_event: KeyEvent,
    key: Key,
    modifiers: Modifiers,
) -> Option<Task<Message>> {
    state.maybe_enter_english_passthrough();

    // Shift+Space must work even in 英 passthrough (otherwise fullwidth can never
    // be enabled from that mode — passthrough would eat the chord).
    if key.as_ref() == Key::Character(" ") && modifiers.shift {
        if state.config.enable_fullwidth_toggle_key {
            state.process_shift_space();
            return Some(state.process_key_and_sync());
        }
    }

    if state.passthrough_mode {
        return None;
    }

    if key.as_ref() == Key::Character(" ") {
        // Bare space with empty buffer: pass through to the client.
        return None;
    }

    if let Key::Character(_) = &key {
        process_ascii_char(state, &key_event)
    } else {
        None
    }
}

/// Watch the chewing settings config file for changes
fn config_watcher() -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::stream::channel(10, async |mut sender| {
        use cosmic::iced::futures::SinkExt;
        use tokio::sync::mpsc as tokio_mpsc;

        let config =
            match cosmic::cosmic_config::Config::new(settings::CONFIG_NAME, settings::CONFIG_VERSION)
            {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to open config for watching: {}", e);
                    std::future::pending::<()>().await;
                    unreachable!()
                }
            };

        let (tx, mut rx) = tokio_mpsc::unbounded_channel::<()>();

        let _watcher = match config.watch(move |_, _| {
            let _ = tx.send(());
        }) {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to create config watcher: {}", e);
                std::future::pending::<()>().await;
                unreachable!()
            }
        };

        while rx.recv().await.is_some() {
            let _ = sender.send(Message::ConfigChanged).await;
        }
    })
}
