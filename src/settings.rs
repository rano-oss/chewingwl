//! Chewing IME settings window
//!
//! Launched via `chewingwl --settings`. Uses full libcosmic Application
//! for proper cosmic theme (rounded corners, system colors, etc).

use cosmic::app::{Core, Task};
use cosmic::iced::{self, Length};
use cosmic::widget::{self, column, container, toggler};
use cosmic::{executor, Element};
use serde::{Deserialize, Serialize};

pub const CONFIG_NAME: &str = "com.chewingwl.Settings";
pub const CONFIG_VERSION: u64 = 1;

/// Selection key presets (same as ibus-chewing)
const SELECTION_KEY_PRESETS: &[&str] = &[
    "1234567890",
    "asdfghjkl;",
    "asdfzxcv89",
    "asdfjkl789",
    "aoeu;qjkix",
    "aoeuhtnsid",
    "aoeuidhtns",
    "1234qweras",
];

const KEYBOARD_LAYOUTS: &[&str] = &[
    "Standard",
    "Hsu",
    "Ibm",
    "GinYieh",
    "Et",
    "Et26",
    "Dvorak",
    "DvorakHsu",
    "DachenCp26",
    "HanyuPinyin",
    "ThlPinyin",
    "Mps2Pinyin",
    "Carpalx",
    "ColemakDhAnsi",
    "ColemakDhOrth",
    "Workman",
];

const ENGINE_VALUES: &[&str] = &["Simple", "Chewing", "FuzzyChewing"];
const ENGINE_LABELS: &[&str] = &["Plain Zhuyin", "Chewing (Default)", "Fuzzy Chewing"];

const CHI_ENG_VALUES: &[&str] = &["Disable", "CapsLock", "Shift", "ShiftL", "ShiftR"];
const CHI_ENG_LABELS: &[&str] = &["Disable", "Caps Lock", "Shift", "Shift_L", "Shift_R"];

const SYNC_CAPS_VALUES: &[&str] = &["Disable", "Keyboard"];
const SYNC_CAPS_LABELS: &[&str] = &["Disable Syncing", "Sync with Keyboard State"];

const ENGLISH_CASE_VALUES: &[&str] = &["NoDefault", "Lowercase", "Uppercase"];
const ENGLISH_CASE_LABELS: &[&str] = &["No Default", "Lowercase", "Uppercase"];

const SWITCH_IM_VALUES: &[&str] = &["Clear", "Keep", "CommitPreedit", "CommitDefault"];
const SWITCH_IM_LABELS: &[&str] = &[
    "Clear preedit",
    "Keep preedit",
    "Commit preedit",
    "Commit default selection",
];

/// Chewing settings stored in cosmic-config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChewingConfig {
    pub keyboard_layout: String,
    pub selection_keys: String,
    pub conversion_engine: String,
    pub candidates_per_page: usize,
    pub auto_commit_threshold: usize,
    pub space_is_select_key: bool,
    pub esc_clear_all_buffer: bool,
    pub auto_shift_cursor: bool,
    pub add_phrase_direction: bool,
    pub phrase_choice_rearward: bool,
    pub disable_auto_learn_phrase: bool,
    pub clean_buffer_focus_out: bool,
    pub easy_symbol_input: bool,
    pub enable_fullwidth_toggle_key: bool,
    pub default_fullwidth: bool,
    pub default_use_english_mode: bool,
    pub chi_eng_mode_toggle: String,
    pub sync_caps_lock: String,
    pub default_english_case: String,
    pub show_page_number: bool,
    pub vertical_lookup_table: bool,
    pub notify_mode_change: bool,
    #[serde(default)]
    pub select_candidate_with_arrow_key: bool,
    #[serde(default)]
    pub use_keypad_as_selection_key: bool,
    /// Clear | Keep | CommitPreedit | CommitDefault (fcitx-style). `Keep` matches
    /// legacy `clean_buffer_focus_out = false`.
    #[serde(default = "default_switch_im")]
    pub switch_im_behavior: String,
}

fn default_switch_im() -> String {
    "Clear".into()
}

impl Default for ChewingConfig {
    fn default() -> Self {
        Self {
            keyboard_layout: "Standard".to_string(),
            selection_keys: "1234567890".to_string(),
            conversion_engine: "Chewing".to_string(),
            candidates_per_page: 10,
            auto_commit_threshold: 20,
            space_is_select_key: false,
            esc_clear_all_buffer: true,
            auto_shift_cursor: false,
            add_phrase_direction: true,
            phrase_choice_rearward: false,
            disable_auto_learn_phrase: false,
            clean_buffer_focus_out: true,
            easy_symbol_input: true,
            enable_fullwidth_toggle_key: true,
            default_fullwidth: false,
            default_use_english_mode: false,
            chi_eng_mode_toggle: "Shift".to_string(),
            sync_caps_lock: "Disable".to_string(),
            default_english_case: "NoDefault".to_string(),
            show_page_number: false,
            vertical_lookup_table: true,
            notify_mode_change: false,
            select_candidate_with_arrow_key: true,
            use_keypad_as_selection_key: false,
            switch_im_behavior: "Clear".to_string(),
        }
    }
}

impl ChewingConfig {
    pub fn load() -> Self {
        let config = match cosmic::cosmic_config::Config::new(CONFIG_NAME, CONFIG_VERSION) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        use cosmic::cosmic_config::ConfigGet;
        let mut cfg: Self = config.get("settings").unwrap_or_default();
        // Migrate older configs that only had clean_buffer_focus_out.
        if !SWITCH_IM_VALUES.contains(&cfg.switch_im_behavior.as_str()) {
            cfg.switch_im_behavior = if cfg.clean_buffer_focus_out {
                "Clear".into()
            } else {
                "Keep".into()
            };
        }
        cfg
    }

    pub fn save(&self) {
        let config = match cosmic::cosmic_config::Config::new(CONFIG_NAME, CONFIG_VERSION) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to open config for save: {}", e);
                return;
            }
        };
        use cosmic::cosmic_config::ConfigSet;
        if let Err(e) = config.set("settings", self) {
            log::error!("Failed to save settings: {}", e);
        }
    }
}

// --- Settings Window App ---

struct SettingsApp {
    core: Core,
    config: ChewingConfig,
    status: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    SetKeyboardLayout(String),
    SetSelectionKeys(String),
    SetConversionEngine(String),
    SetChiEngModeToggle(String),
    SetSyncCapsLock(String),
    SetDefaultEnglishCase(String),
    SetCandidatesPerPage(usize),
    SetAutoCommitThreshold(usize),
    ToggleSpaceIsSelect(bool),
    ToggleEscClearAll(bool),
    ToggleAutoShiftCursor(bool),
    ToggleAddPhraseDirection(bool),
    TogglePhraseChoiceRearward(bool),
    ToggleDisableAutoLearn(bool),
    ToggleCleanBufferFocusOut(bool),
    ToggleEasySymbol(bool),
    ToggleFullwidthKey(bool),
    ToggleDefaultFullwidth(bool),
    ToggleDefaultUseEnglish(bool),
    ToggleShowPageNumber(bool),
    ToggleVerticalLookupTable(bool),
    ToggleNotifyModeChange(bool),
    ToggleSelectWithArrowKey(bool),
    ToggleKeypadAsSelection(bool),
    SetSwitchImBehavior(String),
    ExportUserDict,
    ImportUserDict,
    ClearUserDict,
    OpenUserDictFolder,
}

impl cosmic::Application for SettingsApp {
    type Executor = executor::Default;
    type Flags = ();
    type Message = Msg;

    const APP_ID: &'static str = "com.chewingwl.Settings";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let config = ChewingConfig::load();
        let app = SettingsApp {
            core,
            config,
            status: None,
        };
        (app, Task::none())
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![widget::text::title3("Chewing Settings").into()]
    }

    fn update(&mut self, msg: Self::Message) -> Task<Self::Message> {
        self.status = apply_msg(&mut self.config, msg);
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        settings_view(&self.config, self.status.as_deref())
    }
}

/// Apply a settings message and persist. Returns status for user-dict actions.
pub fn apply_msg(config: &mut ChewingConfig, msg: Msg) -> Option<String> {
    let mut status = None;
    match msg {
        Msg::SetKeyboardLayout(l) => config.keyboard_layout = l,
        Msg::SetSelectionKeys(k) => config.selection_keys = k,
        Msg::SetConversionEngine(e) => config.conversion_engine = e,
        Msg::SetChiEngModeToggle(t) => config.chi_eng_mode_toggle = t,
        Msg::SetSyncCapsLock(s) => config.sync_caps_lock = s,
        Msg::SetDefaultEnglishCase(c) => config.default_english_case = c,
        Msg::SetCandidatesPerPage(n) => config.candidates_per_page = n.clamp(1, 10),
        Msg::SetAutoCommitThreshold(n) => config.auto_commit_threshold = n.clamp(11, 39),
        Msg::ToggleSpaceIsSelect(v) => config.space_is_select_key = v,
        Msg::ToggleEscClearAll(v) => config.esc_clear_all_buffer = v,
        Msg::ToggleAutoShiftCursor(v) => config.auto_shift_cursor = v,
        Msg::ToggleAddPhraseDirection(v) => config.add_phrase_direction = v,
        Msg::TogglePhraseChoiceRearward(v) => config.phrase_choice_rearward = v,
        Msg::ToggleDisableAutoLearn(v) => config.disable_auto_learn_phrase = v,
        Msg::ToggleCleanBufferFocusOut(v) => {
            config.clean_buffer_focus_out = v;
            config.switch_im_behavior = if v {
                "Clear".into()
            } else {
                "Keep".into()
            };
        }
        Msg::ToggleEasySymbol(v) => config.easy_symbol_input = v,
        Msg::ToggleFullwidthKey(v) => config.enable_fullwidth_toggle_key = v,
        Msg::ToggleDefaultFullwidth(v) => config.default_fullwidth = v,
        Msg::ToggleDefaultUseEnglish(v) => config.default_use_english_mode = v,
        Msg::ToggleShowPageNumber(v) => config.show_page_number = v,
        Msg::ToggleVerticalLookupTable(v) => config.vertical_lookup_table = v,
        Msg::ToggleNotifyModeChange(v) => config.notify_mode_change = v,
        Msg::ToggleSelectWithArrowKey(v) => config.select_candidate_with_arrow_key = v,
        Msg::ToggleKeypadAsSelection(v) => config.use_keypad_as_selection_key = v,
        Msg::SetSwitchImBehavior(v) => {
            config.switch_im_behavior = v;
            config.clean_buffer_focus_out = config.switch_im_behavior == "Clear";
        }
        Msg::ExportUserDict => status = Some(export_user_dict()),
        Msg::ImportUserDict => status = Some(import_user_dict()),
        Msg::ClearUserDict => status = Some(clear_user_dict()),
        Msg::OpenUserDictFolder => status = Some(open_user_dict_folder()),
    }
    config.save();
    status
}

fn index_of(options: &[&str], current: &str) -> Option<usize> {
    options.iter().position(|v| *v == current)
}

/// Settings form body (shared by standalone `--settings` and in-IME window).
pub fn settings_view<'a>(
    config: &'a ChewingConfig,
    status: Option<&str>,
) -> Element<'a, Msg> {
    let spacing = cosmic::theme::spacing();

    let general = widget::settings::section()
        .title("General")
        .add(widget::settings::item(
            "Keyboard layout",
            widget::dropdown(
                KEYBOARD_LAYOUTS,
                index_of(KEYBOARD_LAYOUTS, &config.keyboard_layout),
                |i| Msg::SetKeyboardLayout(KEYBOARD_LAYOUTS[i].to_string()),
            ),
        ))
        .add(widget::settings::item(
            "Selection keys",
            widget::dropdown(
                SELECTION_KEY_PRESETS,
                index_of(SELECTION_KEY_PRESETS, &config.selection_keys),
                |i| Msg::SetSelectionKeys(SELECTION_KEY_PRESETS[i].to_string()),
            ),
        ))
        .add(widget::settings::item(
            "Conversion engine",
            widget::dropdown(ENGINE_LABELS, index_of(ENGINE_VALUES, &config.conversion_engine), |i| {
                Msg::SetConversionEngine(ENGINE_VALUES[i].to_string())
            }),
        ))
        .add(widget::settings::item(
            "Chinese / alphanumeric toggle key",
            widget::dropdown(
                CHI_ENG_LABELS,
                index_of(CHI_ENG_VALUES, &config.chi_eng_mode_toggle),
                |i| Msg::SetChiEngModeToggle(CHI_ENG_VALUES[i].to_string()),
            ),
        ))
        .add(widget::settings::item(
            "Sync Caps Lock and IM",
            widget::dropdown(
                SYNC_CAPS_LABELS,
                index_of(SYNC_CAPS_VALUES, &config.sync_caps_lock),
                |i| Msg::SetSyncCapsLock(SYNC_CAPS_VALUES[i].to_string()),
            ),
        ))
        .add(widget::settings::item(
            "Default English case",
            widget::dropdown(
                ENGLISH_CASE_LABELS,
                index_of(ENGLISH_CASE_VALUES, &config.default_english_case),
                |i| Msg::SetDefaultEnglishCase(ENGLISH_CASE_VALUES[i].to_string()),
            ),
        ))
        .add(widget::settings::item(
            "On focus out / switch away",
            widget::dropdown(
                SWITCH_IM_LABELS,
                index_of(SWITCH_IM_VALUES, &config.switch_im_behavior),
                |i| Msg::SetSwitchImBehavior(SWITCH_IM_VALUES[i].to_string()),
            ),
        ));

    let candidates = config.candidates_per_page.clamp(1, 10);
    let threshold = config.auto_commit_threshold.clamp(11, 39);

    let numeric = widget::settings::section()
        .title("Candidates")
        .add(widget::settings::item(
            format!("Candidates per page ({candidates})"),
            widget::slider(1.0..=10.0, candidates as f32, |v| {
                Msg::SetCandidatesPerPage(v.round() as usize)
            })
            .width(Length::Fixed(200.0)),
        ))
        .add(widget::settings::item(
            format!("Auto-commit threshold ({threshold})"),
            widget::slider(11.0..=39.0, threshold as f32, |v| {
                Msg::SetAutoCommitThreshold(v.round() as usize)
            })
            .width(Length::Fixed(200.0)),
        ));

    let input_behavior = widget::settings::section()
        .title("Input behavior")
        .add(settings_toggler(
            "Auto-shift cursor",
            config.auto_shift_cursor,
            Msg::ToggleAutoShiftCursor,
        ))
        .add(settings_toggler(
            "Add phrase before cursor",
            config.add_phrase_direction,
            Msg::ToggleAddPhraseDirection,
        ))
        .add(settings_toggler(
            "Easy symbol input",
            config.easy_symbol_input,
            Msg::ToggleEasySymbol,
        ))
        .add(settings_toggler(
            "ESC clears all buffer",
            config.esc_clear_all_buffer,
            Msg::ToggleEscClearAll,
        ))
        .add(settings_toggler(
            "Enable fullwidth toggle (Shift+Space)",
            config.enable_fullwidth_toggle_key,
            Msg::ToggleFullwidthKey,
        ))
        .add(settings_toggler(
            "Default fullwidth mode",
            config.default_fullwidth,
            Msg::ToggleDefaultFullwidth,
        ))
        .add(settings_toggler(
            "Default alphanumeric mode",
            config.default_use_english_mode,
            Msg::ToggleDefaultUseEnglish,
        ))
        .add(settings_toggler(
            "Disable auto-learn phrase",
            config.disable_auto_learn_phrase,
            Msg::ToggleDisableAutoLearn,
        ));

    let selection = widget::settings::section()
        .title("Selection")
        .add(settings_toggler(
            "Space as selection key",
            config.space_is_select_key,
            Msg::ToggleSpaceIsSelect,
        ))
        .add(settings_toggler(
            "Select candidate with arrow keys",
            config.select_candidate_with_arrow_key,
            Msg::ToggleSelectWithArrowKey,
        ))
        .add(settings_toggler(
            "Use keypad as selection keys",
            config.use_keypad_as_selection_key,
            Msg::ToggleKeypadAsSelection,
        ))
        .add(settings_toggler(
            "Choose phrases backwards",
            config.phrase_choice_rearward,
            Msg::TogglePhraseChoiceRearward,
        ))
        .add(settings_toggler(
            "Show page number",
            config.show_page_number,
            Msg::ToggleShowPageNumber,
        ))
        .add(settings_toggler(
            "Vertical candidate panel",
            config.vertical_lookup_table,
            Msg::ToggleVerticalLookupTable,
        ));

    let notifications = widget::settings::section()
        .title("Notifications")
        .add(settings_toggler(
            "Notify mode change (中/英/全)",
            config.notify_mode_change,
            Msg::ToggleNotifyModeChange,
        ));

    let userphrase = chewing::path::userphrase_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(not found)".into());
    let mut user_dict = widget::settings::section()
        .title("User Dictionary")
        .add(widget::settings::item(
            "Path",
            widget::text::body(userphrase),
        ))
        .add(widget::settings::item(
            "",
            widget::row::with_children(vec![
                widget::button::standard("Export").on_press(Msg::ExportUserDict).into(),
                widget::button::standard("Import").on_press(Msg::ImportUserDict).into(),
                widget::button::standard("Open Folder")
                    .on_press(Msg::OpenUserDictFolder)
                    .into(),
                widget::button::destructive("Clear All")
                    .on_press(Msg::ClearUserDict)
                    .into(),
            ])
            .spacing(spacing.space_xs),
        ));
    if let Some(status) = status {
        user_dict = user_dict.add(widget::text::caption(status.to_string()));
    }

    let content = column![
        widget::text::title3("Chewing Settings"),
        general,
        numeric,
        input_behavior,
        selection,
        notifications,
        user_dict,
    ]
    .spacing(spacing.space_m)
    .padding(spacing.space_l)
    .width(Length::Fill);

    // Full-window opaque fill. App clear color must stay transparent (shared with
    // the IM popup), so settings has to paint every pixel itself.
    let bg_color = {
        let theme = cosmic::theme::active();
        iced::Color::from(theme.cosmic().bg_color())
    };
    container(
        widget::scrollable(content)
            .height(Length::Fill)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .class(cosmic::theme::Container::custom(move |_| container::Style {
        background: Some(iced::Background::Color(bg_color)),
        text_color: None,
        ..Default::default()
    }))
    .into()
}

fn settings_toggler<'a>(label: &'a str, value: bool, msg: fn(bool) -> Msg) -> Element<'a, Msg> {
    widget::settings::item(label, toggler(value).on_toggle(msg)).into()
}

fn export_user_dict() -> String {
    let Some(path) = chewing::path::userphrase_path() else {
        return "User phrase path not found".into();
    };
    let dict = match chewing::dictionary::UserDictionaryLoader::new()
        .userphrase_path(&path)
        .load()
    {
        Ok(d) => d,
        Err(e) => return format!("Failed to open user dict: {e}"),
    };
    let export = path.with_extension("export.txt");
    let mut lines = Vec::new();
    for (syllables, phrase) in dict.entries() {
        let syl: String = syllables
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("{}\t{}\t{}", phrase.as_str(), phrase.freq(), syl));
    }
    match std::fs::write(&export, lines.join("\n")) {
        Ok(()) => format!("Exported {} phrases to {}", lines.len(), export.display()),
        Err(e) => format!("Export failed: {e}"),
    }
}

fn import_user_dict() -> String {
    let Some(path) = chewing::path::userphrase_path() else {
        return "User phrase path not found".into();
    };
    let import = path.with_extension("export.txt");
    let text = match std::fs::read_to_string(&import) {
        Ok(t) => t,
        Err(e) => return format!("Read {}: {e}", import.display()),
    };
    let mut dict = match chewing::dictionary::UserDictionaryLoader::new()
        .userphrase_path(&path)
        .load()
    {
        Ok(d) => d,
        Err(e) => return format!("Failed to open user dict: {e}"),
    };
    let Some(writer) = dict.as_dict_mut() else {
        return "User dictionary is read-only".into();
    };
    let mut n = 0usize;
    for line in text.lines() {
        let mut parts = line.split('\t');
        let Some(phrase) = parts.next() else { continue };
        let freq: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        let Some(syl_str) = parts.next() else { continue };
        let syllables: Vec<chewing::zhuyin::Syllable> = syl_str
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if syllables.is_empty() {
            continue;
        }
        if writer
            .add_phrase(&syllables, (phrase, freq).into())
            .is_ok()
        {
            n += 1;
        }
    }
    let _ = writer.flush();
    format!("Imported {n} phrases from {}", import.display())
}

fn clear_user_dict() -> String {
    let Some(path) = chewing::path::userphrase_path() else {
        return "User phrase path not found".into();
    };
    let mut dict = match chewing::dictionary::UserDictionaryLoader::new()
        .userphrase_path(&path)
        .load()
    {
        Ok(d) => d,
        Err(e) => return format!("Failed to open user dict: {e}"),
    };
    let entries: Vec<_> = dict
        .entries()
        .map(|(s, p)| (s.to_vec(), p.as_str().to_string()))
        .collect();
    let Some(writer) = dict.as_dict_mut() else {
        return "User dictionary is read-only".into();
    };
    let mut n = 0usize;
    for (syllables, phrase) in &entries {
        if writer.remove_phrase(syllables, phrase).is_ok() {
            n += 1;
        }
    }
    let _ = writer.flush();
    format!("Removed {n} phrases (restart IME to fully reload)")
}

fn open_user_dict_folder() -> String {
    let Some(path) = chewing::path::userphrase_path() else {
        return "User phrase path not found".into();
    };
    let dir = path.parent().unwrap_or(path.as_path());
    match std::process::Command::new("xdg-open").arg(dir).spawn() {
        Ok(_) => format!("Opened {}", dir.display()),
        Err(e) => format!("Failed to open folder: {e}"),
    }
}

pub fn run_settings() -> iced::Result {
    let settings = cosmic::app::Settings::default().size(iced::Size::new(600.0, 800.0));
    cosmic::app::run::<SettingsApp>(settings, ())
}
