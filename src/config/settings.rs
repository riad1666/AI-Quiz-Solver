use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_CONFIG_VERSION: u32 = 4;
pub const CUSTOM_MODEL_OPTION: &str = "Custom...";
pub const DEFAULT_VISION_MODEL: &str = "z-ai/glm-5.3-flash";
pub const DEFAULT_REASONING_MODEL: &str = "openai/gpt-4o-mini";
pub const DEFAULT_COPY_EXPLANATIONS_HOTKEY: &str = "Ctrl+Alt+C";

pub const VISION_MODEL_PRESETS: &[&str] = &[
    DEFAULT_VISION_MODEL,
    "anthropic/claude-fable-5.2",
    "anthropic/claude-opus-5.5",
    "openai/gpt-6.1-terra",
    "openai/gpt-6.1-sol",
    "openai/gpt-6-astra",
    "google/gemini-4-carbon",
    "google/gemini-2.5-flash",
    "qwen/qwen4-max",
    "x-ai/grok-4.7",
    CUSTOM_MODEL_OPTION,
];

pub const REASONING_MODEL_PRESETS: &[&str] = &[
    DEFAULT_REASONING_MODEL,
    "anthropic/claude-fable-5.2",
    "anthropic/claude-opus-5.5",
    "openai/gpt-6-astra",
    "openai/gpt-6.1-sol",
    "google/gemini-4-argon",
    "google/gemini-4-carbon",
    "deepseek/deepseek-v4-pro-0813",
    "deepseek/deepseek-v4.1-flash",
    "qwen/qwen4-max",
    "z-ai/glm-5.3-prime",
    "x-ai/grok-4.7",
    CUSTOM_MODEL_OPTION,
];

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("failed to determine application config directory")]
    MissingConfigDir,
    #[error("I/O error reading/writing settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse TOML settings: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize TOML settings: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub config_version: u32,
    pub hotkey: String,
    pub qa_toggle_hotkey: String,
    pub copy_answers_hotkey: String,
    pub copy_explanations_hotkey: String,
    pub vision_model: String,
    pub reasoning_model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub request_timeout_secs: u64,
    pub retry_count: u32,
    pub auto_fill_enabled: bool,
    pub confidence_threshold: f32,
    pub show_tray_notifications: bool,
    pub selection_visual: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            hotkey: "Alt+Z".to_string(),
            qa_toggle_hotkey: "Alt+X".to_string(),
            copy_answers_hotkey: "Alt+C".to_string(),
            copy_explanations_hotkey: DEFAULT_COPY_EXPLANATIONS_HOTKEY.to_string(),
            vision_model: DEFAULT_VISION_MODEL.to_string(),
            reasoning_model: DEFAULT_REASONING_MODEL.to_string(),
            max_tokens: 2048,
            temperature: 0.1,
            request_timeout_secs: 30,
            retry_count: 2,
            auto_fill_enabled: true,
            confidence_threshold: 0.75,
            show_tray_notifications: true,
            selection_visual: "none".to_string(),
        }
    }
}

impl AppSettings {
    /// Returns the default settings file path under `%APPDATA%\KNUDiagnosis\settings.toml`.
    pub fn default_path() -> Result<PathBuf, SettingsError> {
        let app_data = std::env::var_os("APPDATA").ok_or(SettingsError::MissingConfigDir)?;
        Ok(PathBuf::from(app_data)
            .join("KNUDiagnosis")
            .join("settings.toml"))
    }

    /// Loads settings from `path`, creating the file with defaults if it does not exist.
    pub fn load_or_create(path: &Path) -> Result<Self, SettingsError> {
        if !path.exists() {
            let defaults = Self::default();
            defaults.save(path)?;
            return Ok(defaults);
        }

        let raw = fs::read_to_string(path)?;
        // Parse with config_version defaulting to 1 if absent in older settings files.
        let mut parsed: Self = toml::from_str(&raw)?;
        if !raw.contains("config_version") {
            parsed.config_version = 1;
        }
        if parsed.config_version < CURRENT_CONFIG_VERSION {
            if parsed.config_version < 3 {
                if parsed.hotkey.eq_ignore_ascii_case("F8") {
                    parsed.hotkey = "Alt+Z".to_string();
                }
                if parsed.qa_toggle_hotkey.eq_ignore_ascii_case("F9") {
                    parsed.qa_toggle_hotkey = "Alt+X".to_string();
                }
                if parsed.vision_model == "google/gemini-2.5-flash" {
                    parsed.vision_model = DEFAULT_VISION_MODEL.to_string();
                }
            }
            if parsed.copy_answers_hotkey.trim().is_empty() {
                parsed.copy_answers_hotkey = "Alt+C".to_string();
            }
            if parsed.copy_explanations_hotkey.trim().is_empty() {
                parsed.copy_explanations_hotkey = DEFAULT_COPY_EXPLANATIONS_HOTKEY.to_string();
            }
            parsed.config_version = CURRENT_CONFIG_VERSION;
            parsed.save(path)?;
        }
        Ok(parsed)
    }

    /// Persists settings to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)?;
        fs::write(path, serialized)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_settings_toml_and_v1_migration() {
        let temp_dir = std::env::temp_dir().join("knu_diagnosis_test_settings");
        let _ = fs::remove_dir_all(&temp_dir);
        let path = temp_dir.join("settings.toml");

        let mut settings = AppSettings::default();
        assert_eq!(settings.hotkey, "Alt+Z");
        assert_eq!(settings.qa_toggle_hotkey, "Alt+X");
        assert_eq!(settings.copy_answers_hotkey, "Alt+C");
        assert_eq!(settings.vision_model, "z-ai/glm-5.3-flash");

        settings.hotkey = "Ctrl+Shift+Z".to_string();
        settings.confidence_threshold = 0.85;
        settings.save(&path).expect("save settings");

        let loaded = AppSettings::load_or_create(&path).expect("load settings");
        assert_eq!(loaded.hotkey, "Ctrl+Shift+Z");
        assert!((loaded.confidence_threshold - 0.85).abs() < f32::EPSILON);

        // Verify v1 legacy defaults migrate cleanly to Alt+Z / Alt+X / Alt+C and z-ai/glm-5.3-flash.
        fs::write(
            &path,
            "hotkey = \"F8\"\nqa_toggle_hotkey = \"F9\"\nvision_model = \"google/gemini-2.5-flash\"\n",
        )
        .expect("write v1 settings");
        let migrated = AppSettings::load_or_create(&path).expect("migrate v1 settings");
        assert_eq!(migrated.hotkey, "Alt+Z");
        assert_eq!(migrated.qa_toggle_hotkey, "Alt+X");
        assert_eq!(migrated.copy_answers_hotkey, "Alt+C");
        assert_eq!(migrated.copy_explanations_hotkey, "Ctrl+Alt+C");
        assert_eq!(migrated.vision_model, "z-ai/glm-5.3-flash");
        assert_eq!(migrated.reasoning_model, "openai/gpt-4o-mini");
        assert_eq!(migrated.config_version, CURRENT_CONFIG_VERSION);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn model_presets_retain_defaults_include_frontier_models_and_end_with_custom() {
        assert_eq!(DEFAULT_VISION_MODEL, "z-ai/glm-5.3-flash");
        assert_eq!(DEFAULT_REASONING_MODEL, "openai/gpt-4o-mini");
        assert_eq!(CUSTOM_MODEL_OPTION, "Custom...");
        assert_eq!(DEFAULT_COPY_EXPLANATIONS_HOTKEY, "Ctrl+Alt+C");

        assert_eq!(VISION_MODEL_PRESETS.first().copied(), Some(DEFAULT_VISION_MODEL));
        assert_eq!(VISION_MODEL_PRESETS.last().copied(), Some(CUSTOM_MODEL_OPTION));
        for required in [
            "z-ai/glm-5.3-flash",
            "anthropic/claude-opus-5.5",
            "google/gemini-2.5-flash",
            "anthropic/claude-fable-5.2",
            "openai/gpt-6.1-terra",
            "openai/gpt-6.1-sol",
            "openai/gpt-6-astra",
            "google/gemini-4-carbon",
            "qwen/qwen4-max",
            "x-ai/grok-4.7",
            CUSTOM_MODEL_OPTION,
        ] {
            assert!(
                VISION_MODEL_PRESETS.contains(&required),
                "VISION_MODEL_PRESETS missing {required}"
            );
        }

        assert_eq!(
            REASONING_MODEL_PRESETS.first().copied(),
            Some(DEFAULT_REASONING_MODEL)
        );
        assert_eq!(
            REASONING_MODEL_PRESETS.last().copied(),
            Some(CUSTOM_MODEL_OPTION)
        );
        for required in [
            "openai/gpt-4o-mini",
            "anthropic/claude-fable-5.2",
            "anthropic/claude-opus-5.5",
            "openai/gpt-6-astra",
            "openai/gpt-6.1-sol",
            "google/gemini-4-argon",
            "google/gemini-4-carbon",
            "deepseek/deepseek-v4-pro-0813",
            "deepseek/deepseek-v4.1-flash",
            "qwen/qwen4-max",
            "z-ai/glm-5.3-prime",
            "x-ai/grok-4.7",
            CUSTOM_MODEL_OPTION,
        ] {
            assert!(
                REASONING_MODEL_PRESETS.contains(&required),
                "REASONING_MODEL_PRESETS missing {required}"
            );
        }
    }

    #[test]
    fn v3_migration_preserves_vision_model_and_defaults_copy_explanations_hotkey() {
        let temp_dir = std::env::temp_dir().join("knu_diagnosis_test_v3_migration");
        let _ = fs::remove_dir_all(&temp_dir);
        let path = temp_dir.join("settings.toml");
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        fs::write(
            &path,
            "config_version = 3\nhotkey = \"Alt+Z\"\nqa_toggle_hotkey = \"Alt+X\"\ncopy_answers_hotkey = \"Alt+C\"\nvision_model = \"google/gemini-2.5-flash\"\nreasoning_model = \"deepseek/deepseek-v4-pro-0813\"\n",
        )
        .expect("write v3 settings");

        let migrated = AppSettings::load_or_create(&path).expect("migrate v3 settings");
        assert_eq!(migrated.config_version, CURRENT_CONFIG_VERSION);
        assert_eq!(migrated.vision_model, "google/gemini-2.5-flash");
        assert_eq!(migrated.reasoning_model, "deepseek/deepseek-v4-pro-0813");
        assert_eq!(migrated.copy_explanations_hotkey, "Ctrl+Alt+C");

        // Also test custom copy_explanations_hotkey roundtrip
        let mut custom = migrated;
        custom.copy_explanations_hotkey = "Ctrl+Shift+E".to_string();
        custom.vision_model = "custom/my-vision".to_string();
        custom.reasoning_model = "custom/my-reason".to_string();
        custom.save(&path).expect("save custom settings");

        let reloaded = AppSettings::load_or_create(&path).expect("reload custom settings");
        assert_eq!(reloaded.copy_explanations_hotkey, "Ctrl+Shift+E");
        assert_eq!(reloaded.vision_model, "custom/my-vision");
        assert_eq!(reloaded.reasoning_model, "custom/my-reason");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
