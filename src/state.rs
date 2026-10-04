//! Input method state management and chewing editor lifecycle.

use chewing::{
    conversion::ChewingEngine,
    dictionary::{Layered, SystemDictionaryLoader, UserDictionaryLoader},
    editor::{
        keyboard::{self, AnyKeyboardLayout, KeyboardLayout},
        BasicEditor, Editor, LaxUserFreqEstimate,
    },
};

use cosmic::iced::platform_specific::shell::wayland::commands::input_method::{self};
use cosmic::iced::Task;

use std::cmp::min;

use crate::settings::ChewingConfig;
use crate::Message;

/// Maximum number of page columns shown simultaneously in the expanded view
pub const MAX_VISIBLE_PAGES: usize = 4;

pub struct InputMethodState {
    pub index: usize,
    pub page: usize,
    pub editor: Editor,
    pub keyboard: AnyKeyboardLayout,
    pub shift_set: bool,
    /// After an English-mode commit, keys bypass the editor until 中 is selected
    /// again (or English-in-preedit is re-entered via toggle while composing).
    pub passthrough_mode: bool,
    pub visible_page_offset: usize,
    pub num_visible_pages: usize,
    pub popup_id: cosmic::iced::window::Id,
    pub config: ChewingConfig,
}

impl InputMethodState {
    pub fn publish_mode_status(&self) {
        let english = self.is_english_mode();
        ime_control::publish_mode_label(self.mode_status_text());
        ime_control::publish_menu(ime_control::menu_cn_en_half_full_settings(english));
    }

    pub fn is_english_mode(&self) -> bool {
        self.editor.editor_options().language_mode
            == chewing::editor::LanguageMode::English
    }

    pub fn mode_status_text(&self) -> &str {
        // Passthrough is only used while LanguageMode is English, so 英 covers both
        // in-IME English and post-commit passthrough English.
        if self.is_english_mode() {
            "英"
        } else if self.editor.editor_options().character_form
            == chewing::editor::CharacterForm::Fullwidth
        {
            "全"
        } else {
            "中"
        }
    }

    /// Shift+Space fullwidth toggle via chewing's enable_fullwidth_toggle_key path.
    pub fn process_shift_space(&mut self) {
        let before = self.mode_status_text().to_string();
        self.editor.process_keyevent(
            self.keyboard
                .map_with_mod(keyboard::KeyCode::Space, keyboard::Modifiers::shift()),
        );
        self.sync_passthrough_with_fullwidth();
        self.publish_mode_status();
        self.maybe_notify_mode_change(&before);
    }

    pub fn toggle_fullwidth(&mut self) {
        use chewing::editor::CharacterForm;
        let before = self.mode_status_text().to_string();
        let mut opts = self.editor.editor_options();
        opts.character_form = match opts.character_form {
            CharacterForm::Halfwidth => CharacterForm::Fullwidth,
            CharacterForm::Fullwidth => CharacterForm::Halfwidth,
        };
        self.editor.set_editor_options(opts);
        self.sync_passthrough_with_fullwidth();
        self.publish_mode_status();
        self.maybe_notify_mode_change(&before);
    }

    fn maybe_notify_mode_change(&self, before: &str) {
        let after = self.mode_status_text();
        if before != after && self.config.notify_mode_change {
            let label = after.to_string();
            tokio::spawn(async move {
                if let Err(e) = crate::notify_mode_change(&label).await {
                    log::warn!("Failed to send mode notification: {e}");
                }
            });
        }
    }

    /// After a full commit in English mode, fall through to passthrough 英 —
    /// unless fullwidth is on (passthrough would skip chewing's fullwidth convert).
    pub fn maybe_enter_english_passthrough(&mut self) {
        if self.is_english_mode() && self.preedit().is_empty() && !self.is_fullwidth() {
            self.passthrough_mode = true;
        }
    }

    pub fn is_fullwidth(&self) -> bool {
        self.editor.editor_options().character_form
            == chewing::editor::CharacterForm::Fullwidth
    }

    fn sync_passthrough_with_fullwidth(&mut self) {
        if self.is_fullwidth() {
            self.passthrough_mode = false;
        } else if self.is_english_mode() && self.preedit().is_empty() {
            self.passthrough_mode = true;
        }
    }

    pub fn preedit(&self) -> String {
        let display = self.editor.display();
        let syllable = self.editor.syllable_buffer_display();
        if syllable.is_empty() {
            display.to_string()
        } else {
            format!("{}{}", display, syllable)
        }
    }

    pub fn cursor_byte_position(&self) -> i32 {
        if self.editor.entering_syllable() {
            let display = self.editor.display();
            let syllable = self.editor.syllable_buffer_display();
            return (display.len() + syllable.len()) as i32;
        }

        self.syllable_cursor_to_display_byte_offset(self.editor.cursor()) as i32
    }

    /// Map chewing's syllable-space cursor to a byte offset in the display preedit.
    fn syllable_cursor_to_display_byte_offset(&self, syllable_cursor: usize) -> usize {
        let mut offset = 0;
        for interval in self.editor.intervals() {
            if syllable_cursor <= interval.start {
                break;
            }
            if syllable_cursor < interval.end {
                let rel = syllable_cursor - interval.start;
                let syllables = interval.end - interval.start;
                let chars: Vec<_> = interval.str.chars().collect();
                let char_pos = if syllables == 0 {
                    0
                } else {
                    ((rel * chars.len()) / syllables).min(chars.len())
                };
                offset += chars
                    .iter()
                    .take(char_pos)
                    .map(|c| c.len_utf8())
                    .sum::<usize>();
                return offset;
            }
            offset += interval.str.len();
        }
        offset
    }

    pub fn take_commit(&mut self) -> Option<String> {
        let commit = self.editor.display_commit();
        if commit.is_empty() {
            None
        } else {
            let text = commit.to_owned();
            self.editor.ack();
            Some(text)
        }
    }

    pub fn send_key(&mut self, code: keyboard::KeyCode) {
        self.editor.process_keyevent(self.keyboard.map(code));
    }

    pub fn sync_preedit(&mut self) -> Task<Message> {
        let preedit = self.preedit();
        if preedit.is_empty() {
            Task::batch([
                input_method::reset_popup_size(),
                input_method::set_preedit_string(String::new(), 0, 0),
                input_method::commit(),
            ])
        } else {
            let cursor = self.cursor_byte_position();
            log::debug!(
                "    preedit: {:?} cursor={} selecting={}",
                preedit,
                cursor,
                self.editor.is_selecting()
            );
            Task::batch([
                input_method::set_preedit_string(preedit, cursor, cursor),
                input_method::commit(),
            ])
        }
    }

    pub fn send_key_and_sync(&mut self, code: keyboard::KeyCode) -> Task<Message> {
        self.send_key(code);
        self.process_key_and_sync()
    }

    pub fn send_key_and_preedit(&mut self, code: keyboard::KeyCode) -> Task<Message> {
        self.send_key(code);
        self.sync_preedit()
    }

    pub fn send_commit(&self, text: String) -> Task<Message> {
        log::debug!("    commit: {:?}", text);
        Task::batch([
            input_method::commit_string(text),
            input_method::commit(),
        ])
    }

    pub fn process_key_and_sync(&mut self) -> Task<Message> {
        if let Some(text) = self.take_commit() {
            let preedit = self.preedit();
            if preedit.is_empty() {
                self.maybe_enter_english_passthrough();
                return Task::batch([input_method::reset_popup_size(), self.send_commit(text)]);
            }
            return Task::batch([self.send_commit(text), self.sync_preedit()]);
        }

        if self.editor.is_selecting() {
            self.reset_selection_view();
        } else if self.preedit().is_empty() {
            self.maybe_enter_english_passthrough();
        }
        self.sync_preedit()
    }

    pub fn reset_selection_view(&mut self) {
        self.index = 0;
        self.page = 0;
        self.num_visible_pages = 1;
        self.visible_page_offset = 0;
    }

    /// Get all candidates chunked into visible pages
    pub fn visible_pages(&self) -> Vec<Vec<String>> {
        let all = self.editor.all_candidates().unwrap_or_default();
        if all.is_empty() {
            return Vec::new();
        }
        let per_page = self.config.candidates_per_page;
        let total_pages = all.len().div_ceil(per_page);
        let end_page = min(
            self.visible_page_offset + self.num_visible_pages,
            total_pages,
        );
        all.chunks(per_page)
            .skip(self.visible_page_offset)
            .take(end_page - self.visible_page_offset)
            .map(|chunk| chunk.to_vec())
            .collect()
    }

    pub fn total_pages(&self) -> usize {
        let all = self.editor.all_candidates().unwrap_or_default();
        if all.is_empty() {
            return 0;
        }
        all.len().div_ceil(self.config.candidates_per_page.max(1))
    }

    pub fn select_candidate(&mut self) -> Task<Message> {
        // Sync chewing's internal page to match our tracked page
        let chewing_page = self.editor.current_page_no().unwrap_or(0);
        let diff = self.page as isize - chewing_page as isize;
        let (code, count) = if diff > 0 {
            (keyboard::KeyCode::Right, diff as usize)
        } else {
            (keyboard::KeyCode::Left, (-diff) as usize)
        };
        for _ in 0..count {
            self.send_key(code);
        }
        let _ = self.editor.select(self.index);
        self.index = 0;
        self.page = 0;
        self.process_key_and_sync()
    }

    pub fn apply_config(&mut self, config: &ChewingConfig) {
        use chewing::editor::{
            keyboard::AnyKeyboardLayout,
            zhuyin_layout::{
                DaiChien26, Et, Et26, GinYieh, Hsu, Ibm, Pinyin, Standard, SyllableEditor,
            },
            CharacterForm, ConversionEngineKind, UserPhraseAddDirection,
        };

        let mut opts = self.editor.editor_options();
        opts.candidates_per_page = config.candidates_per_page;
        opts.auto_commit_threshold = config.auto_commit_threshold;
        opts.space_is_select_key = config.space_is_select_key;
        opts.esc_clear_all_buffer = config.esc_clear_all_buffer;
        opts.auto_shift_cursor = config.auto_shift_cursor;
        opts.phrase_choice_rearward = config.phrase_choice_rearward;
        opts.disable_auto_learn_phrase = config.disable_auto_learn_phrase;
        opts.easy_symbol_input = config.easy_symbol_input;
        opts.enable_fullwidth_toggle_key = config.enable_fullwidth_toggle_key;

        // Add phrase direction: true = before cursor (Backward), false = after (Forward)
        opts.user_phrase_add_dir = if config.add_phrase_direction {
            UserPhraseAddDirection::Backward
        } else {
            UserPhraseAddDirection::Forward
        };

        // Conversion engine
        opts.conversion_engine = match config.conversion_engine.as_str() {
            "Simple" => ConversionEngineKind::SimpleEngine,
            "FuzzyChewing" => ConversionEngineKind::FuzzyChewingEngine,
            _ => ConversionEngineKind::ChewingEngine,
        };

        // Preserve runtime 全/半 unless the settings default itself changed.
        if config.default_fullwidth != self.config.default_fullwidth {
            opts.character_form = if config.default_fullwidth {
                CharacterForm::Fullwidth
            } else {
                CharacterForm::Halfwidth
            };
        }

        // language_mode is toggled at runtime (Shift / Caps / panel); only the
        // initial default is applied in main::init via set_english_mode.

        let (keyboard, syllable): (AnyKeyboardLayout, Box<dyn SyllableEditor>) =
            match config.keyboard_layout.as_str() {
                "Hsu" => (AnyKeyboardLayout::qwerty(), Box::new(Hsu::new())),
                "Ibm" => (AnyKeyboardLayout::qwerty(), Box::new(Ibm::new())),
                "GinYieh" => (AnyKeyboardLayout::qwerty(), Box::new(GinYieh::new())),
                "Et" => (AnyKeyboardLayout::qwerty(), Box::new(Et::new())),
                "Et26" => (AnyKeyboardLayout::qwerty(), Box::new(Et26::new())),
                "Dvorak" => (AnyKeyboardLayout::dvorak(), Box::new(Standard::new())),
                "DvorakHsu" => (
                    AnyKeyboardLayout::dvorak_on_qwerty(),
                    Box::new(Hsu::new()),
                ),
                "DachenCp26" => (AnyKeyboardLayout::qwerty(), Box::new(DaiChien26::new())),
                "HanyuPinyin" => (AnyKeyboardLayout::qwerty(), Box::new(Pinyin::hanyu())),
                "ThlPinyin" => (AnyKeyboardLayout::qwerty(), Box::new(Pinyin::thl())),
                "Mps2Pinyin" => (AnyKeyboardLayout::qwerty(), Box::new(Pinyin::mps2())),
                "Carpalx" => (AnyKeyboardLayout::qgmlwy(), Box::new(Standard::new())),
                "ColemakDhAnsi" => (
                    AnyKeyboardLayout::colemak_dh_ansi(),
                    Box::new(Standard::new()),
                ),
                "ColemakDhOrth" => (
                    AnyKeyboardLayout::colemak_dh_orth(),
                    Box::new(Standard::new()),
                ),
                "Workman" => (AnyKeyboardLayout::workman(), Box::new(Standard::new())),
                // "Standard" and unknown
                _ => (AnyKeyboardLayout::qwerty(), Box::new(Standard::new())),
            };
        self.keyboard = keyboard;
        self.editor.set_syllable_editor(syllable);

        self.editor.set_editor_options(opts);
        self.config = config.clone();
    }
}

/// Create a new chewing Editor with system/user dictionaries loaded
pub fn create_editor() -> Editor {
    let sys_loader = SystemDictionaryLoader::new();
    let dictionaries = sys_loader.load().expect("System dictionary not found");
    let user_dictionary = UserDictionaryLoader::new()
        .load()
        .expect("User dictionary not found");
    let abbrev = sys_loader
        .load_abbrev()
        .expect("Failed to load abbreviation table");
    let estimate = LaxUserFreqEstimate::max_from(user_dictionary.as_ref());
    let dict = Layered::new(dictionaries, user_dictionary);
    let conversion_engine = Box::new(ChewingEngine::new());
    let sym_sel = sys_loader
        .load_symbol_selector()
        .expect("Failed to load symbol table");
    Editor::new(conversion_engine, dict, estimate, abbrev, sym_sel)
}
