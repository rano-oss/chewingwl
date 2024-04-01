use chewing::{
    conversion::ChewingEngine,
    dictionary::{LayeredDictionary, SystemDictionaryLoader, UserDictionaryLoader},
    editor::{
        keyboard::{self, AnyKeyboardLayout, KeyboardLayout, Modifiers as Mods, Qwerty},
        // syllable::KeyboardLayoutCompat,
        BasicEditor,
        Editor,
        LaxUserFreqEstimate,
    },
};
use iced::{
    event::{self, listen_raw, wayland::InputMethodEvent},
    keyboard::key::Named,
    wayland::{
        actions::{
            input_method::ActionInner, input_method_popup::InputMethodPopupSettings,
            virtual_keyboard::ActionInner as VKActionInner,
        },
        input_method::{hide_input_method_popup, input_method_action, show_input_method_popup},
        virtual_keyboard::virtual_keyboard_action,
        InitialSurface,
    },
    widget::{column, container, row, text},
    window, Alignment, Application, Color, Command, Element, Event, Settings, Subscription, Theme,
};
use iced_core::{
    event::wayland::{InputMethodKeyboardEvent, KeyEvent, Modifiers, RawModifiers},
    keyboard::Key,
    window::Id,
    Border,
};
use iced_style::application;
use selection_field::widget::selection_field;
use std::{char, cmp::min, fmt::Debug};
mod selection_field;

fn main() -> iced::Result {
    let initial_surface = InputMethodPopupSettings::default();
    let settings = Settings {
        initial_surface: InitialSurface::InputMethodPopup(initial_surface),
        ..Settings::default()
    };
    InputMethod::run(settings)
}

struct Chewing {
    // kb_compat: KeyboardLayoutCompat,
    editor: Editor<ChewingEngine>,
    keyboard: AnyKeyboardLayout,
}

impl Chewing {
    fn new() -> Self {
        let dictionaries = SystemDictionaryLoader::new().load().unwrap_or_default();
        let user_dictionary = UserDictionaryLoader::new().load().unwrap();
        let estimate = LaxUserFreqEstimate::open(user_dictionary.as_ref());
        let dict = LayeredDictionary::new(dictionaries, user_dictionary);
        let engine = ChewingEngine::new();
        // let kb_compat = KeyboardLayoutCompat::Default;
        let keyboard = AnyKeyboardLayout::Qwerty(Qwerty);
        let editor = Editor::new(engine, dict, estimate.unwrap());
        Chewing {
            // kb_compat,
            editor,
            keyboard,
        }
    }

    fn preedit(&self) -> String {
        format!("{}{}", self.editor.display(), self.editor.syllable_buffer())
    }
}

struct InputMethod {
    page: usize,
    index: usize,
    chewing: Chewing,
    state: State,
    candidates: Vec<String>,
    current_preedit: String,
    cursor_position: usize,
    preedit_len: usize,
    pages: Vec<Vec<String>>,
    max_candidates: usize,
    max_pages: usize,
    popup: bool,
    shift_set: bool,
    passthrough_mode: bool,
}

impl InputMethod {
    fn set_cursor_position(&mut self) {
        let chars: Vec<char> = self.current_preedit.chars().collect();
        self.cursor_position = chars[..self.chewing.editor.cursor()]
            .iter()
            .collect::<String>()
            .len()
    }

    fn preedit_string(&mut self) -> Command<Message> {
        let preedit = self.chewing.preedit();
        self.preedit_len = preedit.len();
        self.current_preedit = preedit.clone();
        self.state = State::WaitingForDone;
        self.set_cursor_position();
        Command::batch(vec![
            input_method_action(ActionInner::SetPreeditString {
                string: preedit,
                cursor_begin: self.cursor_position as i32,
                cursor_end: self.cursor_position as i32,
            }),
            input_method_action(ActionInner::Commit),
        ])
    }

    fn commit_string(&mut self) -> Command<Message> {
        let commit_string = self.chewing.preedit();
        self.state = State::PassThrough;
        self.chewing
            .editor
            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Enter));
        Command::batch(vec![
            input_method_action(ActionInner::CommitString(commit_string)),
            input_method_action(ActionInner::Commit),
        ])
    }

    fn open_popup(&mut self) -> Command<Message> {
        let preedit = self.chewing.preedit();
        self.chewing
            .editor
            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Down));
        self.candidates = self.chewing.editor.all_candidates().unwrap_or_default();
        self.state = State::WaitingForDone;
        self.popup = true;
        self.set_cursor_position();
        self.index = 0;
        self.page = 0;
        self.pages =
            vec![self.candidates[0..min(self.max_candidates, self.candidates.len())].to_vec()];
        Command::batch(vec![
            input_method_action(ActionInner::SetPreeditString {
                string: preedit,
                cursor_begin: self.cursor_position as i32,
                cursor_end: self.cursor_position as i32,
            }),
            input_method_action(ActionInner::Commit),
        ])
    }

    fn num_select(&mut self, index: usize) -> Command<Message> {
        let _ = self
            .chewing
            .editor
            .select(self.page * self.max_candidates + index);
        self.current_preedit = self.chewing.preedit();
        self.state = State::WaitingForDone;
        self.popup = false;
        self.set_cursor_position();
        Command::batch(vec![
            input_method_action(ActionInner::SetPreeditString {
                string: self.chewing.preedit(),
                cursor_begin: self.cursor_position as i32,
                cursor_end: self.cursor_position as i32,
            }),
            input_method_action(ActionInner::Commit),
            hide_input_method_popup(),
        ])
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Activate,
    Deactivate,
    KeyPressed(KeyEvent, Key, Modifiers),
    KeyReleased(KeyEvent, Key, Modifiers),
    Modifiers(Modifiers, RawModifiers),
    UpdatePopup { page: usize, index: usize },
    ClosePopup,
    Done,
}

#[derive(Clone, Debug)]
enum State {
    PreEdit,
    Popup,
    WaitingForDone,
    PassThrough,
}

impl Application for InputMethod {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = ();
    type Theme = Theme;

    fn new(_flags: ()) -> (InputMethod, Command<Message>) {
        (
            InputMethod {
                page: 0,
                index: 0,
                chewing: Chewing::new(),
                state: State::PassThrough,
                candidates: Vec::new(),
                current_preedit: String::new(),
                cursor_position: 0,
                preedit_len: 0,
                pages: Vec::new(),
                max_candidates: 10,
                max_pages: 4,
                popup: false,
                shift_set: false,
                passthrough_mode: false,
            },
            Command::none(),
        )
    }

    fn title(&self, _: Id) -> String {
        String::from("InputMethod")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Activate => {
                self.state = State::PassThrough;
                Command::none()
            }
            Message::Deactivate => {
                self.chewing
                    .editor
                    .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Esc));
                self.state = State::PassThrough;
                hide_input_method_popup()
            }
            Message::KeyPressed(key_event, key, modifiers) => match self.state {
                State::PreEdit => match key {
                    Key::Named(Named::Backspace) => {
                        self.chewing.editor.process_keyevent(
                            self.chewing.keyboard.map(keyboard::KeyCode::Backspace),
                        );
                        self.preedit_string()
                    }
                    Key::Named(Named::Space) => {
                        if modifiers.shift {
                            self.chewing.editor.process_keyevent(
                                self.chewing
                                    .keyboard
                                    .map_with_mod(keyboard::KeyCode::Space, Mods::shift()),
                            );
                        } else {
                            self.chewing.editor.process_keyevent(
                                self.chewing.keyboard.map(keyboard::KeyCode::Space),
                            );
                        }
                        self.preedit_string()
                    }
                    Key::Named(Named::Enter) => self.commit_string(),
                    Key::Named(Named::Escape) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Esc));
                        self.preedit_string()
                    }
                    Key::Named(Named::Delete) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Del));
                        self.preedit_string()
                    }
                    Key::Named(Named::ArrowLeft) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Left));
                        self.preedit_string()
                    }
                    Key::Named(Named::ArrowRight) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Right));
                        self.preedit_string()
                    }
                    Key::Named(Named::ArrowDown) => self.open_popup(),
                    Key::Named(Named::ArrowUp) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Up));
                        self.preedit_string()
                    }
                    Key::Named(Named::Tab) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Tab));
                        self.preedit_string()
                    }
                    _ => {
                        if let Some(char) = key_event.utf8.as_ref().and_then(|s| s.chars().last()) {
                            self.chewing
                                .editor
                                .process_keyevent(self.chewing.keyboard.map_ascii(char as u8));
                            self.preedit_string()
                        } else {
                            Command::none()
                        }
                    }
                },
                State::Popup => match key.as_ref() {
                    Key::Character("1") => self.num_select(0),
                    Key::Character("2") => self.num_select(1),
                    Key::Character("3") => self.num_select(2),
                    Key::Character("4") => self.num_select(3),
                    Key::Character("5") => self.num_select(4),
                    Key::Character("6") => self.num_select(5),
                    Key::Character("7") => self.num_select(6),
                    Key::Character("8") => self.num_select(7),
                    Key::Character("9") => self.num_select(8),
                    Key::Character("0") => self.num_select(9),
                    Key::Named(Named::ArrowDown) => {
                        let total_pages = self.chewing.editor.total_page().unwrap();
                        if self.index == min(self.candidates.len(), self.max_candidates) - 1
                            || (self.page == total_pages - 1
                                && self.index == self.candidates.len() % self.max_candidates - 1)
                        {
                            self.chewing.editor.process_keyevent(
                                self.chewing.keyboard.map(keyboard::KeyCode::Down),
                            );
                            self.candidates =
                                self.chewing.editor.all_candidates().unwrap_or_default();
                            self.index = 0;
                            self.page = 0;
                            self.pages = vec![self.candidates
                                [0..min(self.max_candidates, self.candidates.len())]
                                .to_vec()];
                        } else if self.page == total_pages - 1 {
                            self.index =
                                min(self.candidates.len() % self.max_candidates, self.index + 1)
                        } else {
                            self.index += 1
                        }
                        Command::none()
                    }
                    Key::Named(Named::ArrowUp) => {
                        self.index = self.index.saturating_sub(1);
                        Command::none()
                    }
                    Key::Named(Named::ArrowLeft) => {
                        if self.page != 0 && self.page % self.max_pages == 0 {
                            let mut pages = Vec::new();
                            let page_index = self.page / (self.max_pages - 1) - 1;
                            let page_size = self.max_candidates * self.max_pages;
                            for p_i in 0..self.max_pages {
                                let page = self.candidates[p_i * self.max_candidates
                                    + page_index * page_size
                                    ..(p_i + 1) * self.max_candidates + page_index * page_size]
                                    .to_vec();
                                pages.push(page);
                            }
                            self.pages = pages;
                        }
                        self.page = self.page.saturating_sub(1);
                        Command::none()
                    }
                    Key::Named(Named::ArrowRight) => {
                        let total_pages = self.chewing.editor.total_page().unwrap();
                        if total_pages > 1 && (self.page == self.max_pages - 1 || self.page == 0) {
                            let mut pages = Vec::new();
                            let page_index = self.page / (self.max_pages - 1);
                            let num_rows =
                                min(total_pages - self.max_pages * page_index, self.max_pages);
                            let page_size = self.max_candidates * self.max_pages;
                            for p_i in 0..num_rows {
                                let page = self.candidates[p_i * self.max_candidates
                                    + page_index * page_size
                                    ..min(
                                        (p_i + 1) * self.max_candidates + page_index * page_size,
                                        self.candidates.len(),
                                    )]
                                    .to_vec();
                                pages.push(page);
                            }
                            self.pages = pages;
                        }
                        self.page = min(self.page + 1, total_pages - 1);
                        if self.page == total_pages - 1 {
                            self.index =
                                min(self.index, self.candidates.len() % self.max_candidates - 1);
                        }
                        Command::none()
                    }
                    Key::Named(Named::Enter) => {
                        let _ = self
                            .chewing
                            .editor
                            .select(self.page * self.max_candidates + self.index);
                        self.current_preedit = self.chewing.preedit();
                        self.state = State::WaitingForDone;
                        self.popup = false;
                        self.set_cursor_position();
                        Command::batch(vec![
                            input_method_action(ActionInner::SetPreeditString {
                                string: self.chewing.preedit(),
                                cursor_begin: self.cursor_position as i32,
                                cursor_end: self.cursor_position as i32,
                            }),
                            input_method_action(ActionInner::Commit),
                            hide_input_method_popup(),
                        ])
                    }
                    Key::Named(Named::Escape) => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Esc));
                        self.state = State::PreEdit;
                        self.popup = false;
                        self.set_cursor_position();
                        Command::batch(vec![
                            input_method_action(ActionInner::SetPreeditString {
                                string: self.chewing.preedit(),
                                cursor_begin: self.cursor_position as i32,
                                cursor_end: self.cursor_position as i32,
                            }),
                            input_method_action(ActionInner::Commit),
                            hide_input_method_popup(),
                        ])
                    }
                    _ => Command::none(),
                },
                State::WaitingForDone => {
                    // Do nothing if text input client is not ready
                    // TODO: add timer for misbehaving/slow/laggy clients
                    Command::none()
                }
                State::PassThrough => {
                    if self.passthrough_mode {
                        if key == Key::Named(Named::Shift) {
                            self.shift_set = true;
                            Command::none()
                        } else {
                            self.shift_set = false;
                            virtual_keyboard_action(VKActionInner::KeyPressed(key_event))
                        }
                    } else if key == Key::Named(Named::Shift) {
                        self.shift_set = true;
                        Command::none()
                    } else if key == Key::Named(Named::Space) {
                        self.shift_set = false;
                        if modifiers.shift {
                            self.chewing.editor.process_keyevent(
                                self.chewing
                                    .keyboard
                                    .map_with_mod(keyboard::KeyCode::Space, Mods::shift()),
                            );
                            Command::none()
                        } else {
                            virtual_keyboard_action(VKActionInner::KeyPressed(key_event))
                        }
                    } else if let Some(char) =
                        key_event.utf8.as_ref().and_then(|s| s.chars().last())
                    {
                        self.shift_set = false;
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map_ascii(char as u8));
                        if self.chewing.preedit().is_empty() {
                            virtual_keyboard_action(VKActionInner::KeyPressed(key_event))
                        } else {
                            self.preedit_string()
                        }
                    } else {
                        self.shift_set = false;
                        virtual_keyboard_action(VKActionInner::KeyPressed(key_event))
                    }
                }
            },
            Message::KeyReleased(key_event, key, _modifiers) => match self.state {
                State::PassThrough => {
                    if key == Key::Named(Named::Shift) && self.shift_set {
                        self.shift_set = false;
                        self.passthrough_mode = !self.passthrough_mode;
                        Command::none()
                    } else {
                        virtual_keyboard_action(VKActionInner::KeyReleased(key_event))
                    }
                }
                State::PreEdit | State::Popup | State::WaitingForDone => Command::none(),
            },
            Message::Modifiers(_modifiers, raw_modifiers) => {
                virtual_keyboard_action(VKActionInner::Modifiers(raw_modifiers))
            }
            Message::Done => match self.state {
                State::WaitingForDone => {
                    if self.popup {
                        self.state = State::Popup;
                        show_input_method_popup()
                    } else if !self.current_preedit.is_empty() {
                        self.state = State::PreEdit;
                        Command::none()
                    } else {
                        self.state = State::PassThrough;
                        Command::none()
                    }
                }
                State::PreEdit | State::Popup | State::PassThrough => Command::none(),
            },
            Message::UpdatePopup { page, index } => {
                self.page = page;
                self.index = index;
                Command::none()
            }
            Message::ClosePopup => {
                let _ = self
                    .chewing
                    .editor
                    .select(self.page * self.max_candidates + self.index);
                self.current_preedit = self.chewing.preedit();
                self.state = State::WaitingForDone;
                self.popup = false;
                self.set_cursor_position();
                Command::batch(vec![
                    input_method_action(ActionInner::SetPreeditString {
                        string: self.chewing.preedit(),
                        cursor_begin: self.cursor_position as i32,
                        cursor_end: self.cursor_position as i32,
                    }),
                    input_method_action(ActionInner::Commit),
                    hide_input_method_popup(),
                ])
            }
        }
    }

    fn view(&self, _id: window::Id) -> Element<Message> {
        container(
            row(self
                .pages
                .iter()
                .enumerate()
                .map(|(page, list)| {
                    column(
                        list.iter()
                            .enumerate()
                            .map(|(index, char)| {
                                selection_field(
                                    row(vec![
                                        text((index + 1) % 10)
                                            .size(50)
                                            .style(if page != self.page % self.max_pages {
                                                Color::TRANSPARENT
                                            } else {
                                                Color::WHITE
                                            })
                                            .into(),
                                        text(char).size(50).into(),
                                    ])
                                    .align_items(Alignment::Center)
                                    .padding(5.0)
                                    .spacing(4.0),
                                )
                                .set_indexes(page, index)
                                .selected(self.page % self.max_pages, self.index)
                                .on_press(Message::ClosePopup)
                                .on_select(Message::UpdatePopup { page, index })
                                .into()
                            })
                            .collect::<Vec<_>>(),
                    )
                    .spacing(5.0)
                    .padding(5.0)
                    .align_items(Alignment::Center)
                    .into()
                })
                .collect::<Vec<_>>())
            .padding(2.0),
        )
        .padding(5.0)
        .style(<iced_style::Theme as container::StyleSheet>::Style::Custom(
            Box::new(CustomTheme),
        ))
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        listen_raw(|event, status| match (event.clone(), status) {
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
                InputMethodKeyboardEvent::Press(key, key_code, modifiers) => {
                    Some(Message::KeyPressed(key, key_code, modifiers))
                }
                InputMethodKeyboardEvent::Release(key, key_code, modifiers) => {
                    Some(Message::KeyReleased(key, key_code, modifiers))
                }
                InputMethodKeyboardEvent::Repeat(key, key_code, modifiers) => {
                    Some(Message::KeyPressed(key, key_code, modifiers))
                }
                InputMethodKeyboardEvent::Modifiers(modifiers, raw_modifiers) => {
                    Some(Message::Modifiers(modifiers, raw_modifiers))
                }
            },
            _ => None,
        })
    }

    fn style(&self) -> <Self::Theme as application::StyleSheet>::Style {
        <Self::Theme as application::StyleSheet>::Style::Custom(Box::new(CustomTheme))
    }
}

pub struct CustomTheme;

impl container::StyleSheet for CustomTheme {
    type Style = iced::Theme;

    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            border: Border {
                color: Color::from_rgb(1.0, 1.0, 1.0),
                width: 3.0,
                radius: 10.0.into(),
            },
            background: Some(Color::from_rgb(0.0, 0.0, 0.0).into()),
            ..container::Appearance::default()
        }
    }
}

impl iced_style::application::StyleSheet for CustomTheme {
    type Style = iced::Theme;

    fn appearance(&self, _style: &Self::Style) -> application::Appearance {
        iced_style::application::Appearance {
            background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
            icon_color: Color::BLACK,
            text_color: Color::BLACK,
        }
    }
}
