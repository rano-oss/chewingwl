use chewing::{
    conversion::ChewingEngine,
    dictionary::{LayeredDictionary, SystemDictionaryLoader, UserDictionaryLoader},
    editor::{
        keyboard::{self, AnyKeyboardLayout, KeyboardLayout, Modifiers as Mods, Qwerty},
        syllable::KeyboardLayoutCompat,
        BasicEditor, Editor, LaxUserFreqEstimate,
    },
};
use iced::{
    event::{self, listen_raw, wayland::InputMethodEvent},
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
    keyboard::KeyCode,
    window::Id,
};
use iced_style::application;
use selection_field::widget::selection_field;
use std::{cmp::min, fmt::Debug};
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
    dict: LayeredDictionary,
    engine: ChewingEngine,
    kb_compat: KeyboardLayoutCompat,
    editor: Editor<ChewingEngine>,
    keyboard: AnyKeyboardLayout,
}

impl Chewing {
    fn new() -> Self {
        let dictionaries = SystemDictionaryLoader::new().load();
        let user_dictionary = UserDictionaryLoader::new().load();
        let estimate = LaxUserFreqEstimate::open(user_dictionary.unwrap().as_ref());
        let dict =
            LayeredDictionary::new(dictionaries.unwrap_or_default(), user_dictionary.unwrap());
        let engine = ChewingEngine::new();
        let kb_compat = KeyboardLayoutCompat::Default;
        let keyboard = AnyKeyboardLayout::Qwerty(Qwerty);
        let editor = Editor::new(engine, dict, estimate.unwrap());
        Chewing {
            dict,
            engine,
            kb_compat,
            editor,
            keyboard,
        }
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
}

impl InputMethod {
    fn preedit_string(&mut self) -> Command<Message> {
        let preedit = self.chewing.editor.display();
        self.preedit_len = preedit.len();
        if self.current_preedit != preedit
            || self.chewing.editor.cursor() * 3 != self.cursor_position
        {
            self.current_preedit = preedit.clone();
            self.state = State::WaitingForDone;
            self.cursor_position = self.chewing.editor.cursor() * 3;
            Command::batch(vec![
                input_method_action(ActionInner::SetPreeditString {
                    string: preedit,
                    cursor_begin: (self.chewing.editor.cursor() * 3) as i32,
                    cursor_end: (self.chewing.editor.cursor() * 3) as i32,
                }),
                input_method_action(ActionInner::Commit),
            ])
        } else {
            Command::none()
        }
    }

    fn commit_string(&mut self) -> Command<Message> {
        let commit_string = format!("{}{}", self.chewing.buffer(), self.chewing.bopomofo());
        self.state = State::PassThrough;
        self.chewing.enter();
        Command::batch(vec![
            input_method_action(ActionInner::CommitString(commit_string)),
            input_method_action(ActionInner::Commit),
        ])
    }

    fn open_popup(&mut self) -> Command<Message> {
        let preedit = self.chewing.preedit();
        self.chewing.down();
        self.candidates = self
            .chewing
            .list()
            .iter()
            .map(|&s| String::from(s))
            .collect();
        self.state = State::WaitingForDone;
        self.popup = true;
        self.cursor_position = self.chewing.editor.cursor() * 3;
        self.index = 0;
        self.page = 0;
        self.pages =
            vec![self.candidates[0..min(self.max_candidates, self.candidates.len())].to_vec()];
        Command::batch(vec![
            input_method_action(ActionInner::SetPreeditString {
                string: preedit,
                cursor_begin: (self.chewing.editor.cursor() * 3) as i32,
                cursor_end: (self.chewing.editor.cursor() * 3) as i32,
            }),
            input_method_action(ActionInner::Commit),
        ])
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Activate,
    Deactivate,
    KeyPressed(KeyEvent, KeyCode, Modifiers),
    KeyReleased(KeyEvent, KeyCode, Modifiers),
    Modifiers(Modifiers, RawModifiers),
    UpdatePopup { page: usize, index: usize },
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
                max_pages: 3,
                popup: false,
            },
            Command::none(),
        )
    }

    fn title(&self, _: Id) -> String {
        String::from("InputMethod")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Activate => Command::none(),
            Message::Deactivate => {
                self.chewing
                    .editor
                    .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Esc));
                self.state = State::PassThrough;
                hide_input_method_popup()
            }
            Message::KeyPressed(key, key_code, modifiers) => match self.state {
                State::PreEdit => match key_code {
                    // KeyCode::LShift => {
                    //     self.chewing.shift_left();
                    //     self.preedit_string()
                    // }
                    // KeyCode::RShift => {
                    //     self.chewing.shift_right();
                    //     self.preedit_string()
                    // }
                    KeyCode::Backspace => {
                        self.chewing.editor.process_keyevent(
                            self.chewing.keyboard.map(keyboard::KeyCode::Backspace),
                        );
                        self.preedit_string()
                    }
                    KeyCode::Space => {
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
                    KeyCode::Enter => self.commit_string(),
                    KeyCode::Escape => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Esc));
                        self.preedit_string()
                    }
                    KeyCode::Delete => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Del));
                        self.preedit_string()
                    }
                    KeyCode::Left => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Left));
                        self.preedit_string()
                    }
                    KeyCode::Right => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Right));
                        self.preedit_string()
                    }
                    KeyCode::Down => self.open_popup(),
                    KeyCode::Up => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Up));
                        self.preedit_string()
                    }
                    KeyCode::Tab => {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map(keyboard::KeyCode::Tab));
                        self.preedit_string()
                    }
                    _ => {
                        if let Some(char) = key.utf8.as_ref().and_then(|s| s.chars().last()) {
                            self.chewing
                                .editor
                                .process_keyevent(self.chewing.keyboard.map_ascii(char as u8));
                            self.preedit_string()
                        } else {
                            Command::none()
                        }
                    }
                },
                State::Popup => match key_code {
                    KeyCode::Down => {
                        if self.index < min(self.candidates.len(), self.max_candidates) - 1 {
                            self.index += 1;
                        }
                        Command::none()
                    }
                    KeyCode::Up => {
                        if self.index > 0 {
                            self.index -= 1;
                        }
                        Command::none()
                    }
                    KeyCode::Left => {
                        if self.page > 0 {
                            self.page -= 1;
                        }
                        Command::none()
                    }
                    KeyCode::Right => {
                        let num_pages = (self.candidates.len() as f32 / self.max_candidates as f32)
                            .ceil() as usize;
                        if num_pages > 1 {
                            let mut pages = Vec::new();
                            for page_index in 0..min(num_pages, self.max_pages + 1) {
                                let page = self.candidates[page_index * self.max_candidates
                                    ..min(
                                        (page_index + 1) * self.max_candidates,
                                        self.candidates.len(),
                                    )]
                                    .to_vec();
                                pages.push(page);
                            }
                            self.pages = pages;
                            self.page += 1;
                        }
                        Command::none()
                    }
                    KeyCode::Enter => {
                        self.chewing
                            .choose_by_index((self.page * self.max_candidates + self.index) as u8);
                        self.current_preedit = self.chewing.preedit();
                        self.state = State::WaitingForDone;
                        self.popup = false;
                        self.cursor_position = self.chewing.cursor_position() * 3;
                        Command::batch(vec![
                            input_method_action(ActionInner::SetPreeditString {
                                string: self.chewing.preedit(),
                                cursor_begin: self.chewing.cursor_position() * 3,
                                cursor_end: self.chewing.cursor_position() * 3,
                            }),
                            input_method_action(ActionInner::Commit),
                            hide_input_method_popup(),
                        ])
                    }
                    KeyCode::Escape => {
                        self.chewing.esc();
                        self.state = State::PreEdit;
                        self.cursor_position = self.chewing.editor.cursor() * 3;
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
                    Command::none()
                }
                State::PassThrough => {
                    if let Some(char) = key.utf8.as_ref().and_then(|s| s.chars().last()) {
                        self.chewing
                            .editor
                            .process_keyevent(self.chewing.keyboard.map_ascii(char as u8));
                        if self.chewing.preedit().is_empty() {
                            virtual_keyboard_action(VKActionInner::KeyPressed(key))
                        } else {
                            self.state = State::PreEdit;
                            self.preedit_string()
                        }
                    } else {
                        virtual_keyboard_action(VKActionInner::KeyPressed(key))
                    }
                }
            },
            Message::KeyReleased(key, _key_code, modifiers) => match self.state {
                State::PassThrough => virtual_keyboard_action(VKActionInner::KeyReleased(key)),
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
                                            .style(if page != self.page {
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
                                .selected(self.page, self.index)
                                .on_press(Message::Deactivate)
                                .on_select(Message::UpdatePopup { page, index })
                                .into()
                            })
                            .collect(),
                    )
                    .spacing(5.0)
                    .padding(5.0)
                    .align_items(Alignment::Center)
                    .into()
                })
                .collect())
            .padding(2.0),
        )
        .padding(5.0)
        .style(<iced_style::Theme as container::StyleSheet>::Style::Custom(
            Box::new(CustomTheme),
        ))
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        dbg!(&self.state);
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
            border_color: Color::from_rgb(1.0, 1.0, 1.0),
            border_radius: 10.0.into(),
            border_width: 3.0,
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
