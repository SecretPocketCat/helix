use crate::keymap;
use crate::keymap::{merge_keys, KeyTrie};
use helix_loader::merge_toml_values;
use helix_view::document::Mode;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::fs;
use std::io::Error as IOError;
use toml::de::Error as TomlError;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub theme: Option<String>,
    // todo: key might be smt. else?
    pub theme_lang: HashMap<String, String>,
    pub keys: HashMap<Mode, KeyTrie>,
    pub keys_lang: HashMap<String, HashMap<Mode, KeyTrie>>,
    pub editor: helix_view::editor::Config,
    pub editor_lang: HashMap<String, helix_view::editor::Config>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRaw {
    pub theme: Option<String>,
    pub keys: Option<HashMap<Mode, KeyTrie>>,
    pub editor: Option<toml::Value>,
    pub languages: Option<Vec<LanguageConfigRaw>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageConfigRaw {
    pub name: String,
    pub theme: Option<String>,
    pub keys: Option<HashMap<Mode, KeyTrie>>,
    pub editor: Option<toml::Value>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            theme: None,
            theme_lang: HashMap::new(),
            keys: keymap::default(),
            keys_lang: HashMap::new(),
            editor: helix_view::editor::Config::default(),
            editor_lang: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigLoadError {
    BadConfig(TomlError),
    Error(IOError),
}

impl Default for ConfigLoadError {
    fn default() -> Self {
        ConfigLoadError::Error(IOError::new(std::io::ErrorKind::NotFound, "place holder"))
    }
}

impl Display for ConfigLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigLoadError::BadConfig(err) => err.fmt(f),
            ConfigLoadError::Error(err) => err.fmt(f),
        }
    }
}

impl Config {
    pub fn load(
        global: Result<String, ConfigLoadError>,
        local: Result<String, ConfigLoadError>,
    ) -> Result<Config, ConfigLoadError> {
        let global_config: Result<ConfigRaw, ConfigLoadError> =
            global.and_then(|file| toml::from_str(&file).map_err(ConfigLoadError::BadConfig));
        let local_config: Result<ConfigRaw, ConfigLoadError> =
            local.and_then(|file| toml::from_str(&file).map_err(ConfigLoadError::BadConfig));

        let res = match (global_config, local_config) {
            (Ok(mut global), Ok(mut local)) => {
                let keys = Self::merge_config_keys(keymap::default(), global.keys, local.keys);

                let editor_value = Self::merge_editor_toml(global.editor, local.editor);

                let (theme_lang, keys_lang, editor_lang) = Self::get_lang_maps(
                    Self::get_lang_config_map(global.languages.take()),
                    Self::get_lang_config_map(local.languages.take()),
                    &keys,
                    &editor_value,
                )?;

                let editor = Self::map_editor_config(editor_value)?;

                Config {
                    theme: local.theme.or(global.theme),
                    keys,
                    editor,
                    theme_lang,
                    keys_lang,
                    editor_lang,
                }
            }
            // if any configs are invalid return that first
            (_, Err(ConfigLoadError::BadConfig(err)))
            | (Err(ConfigLoadError::BadConfig(err)), _) => {
                return Err(ConfigLoadError::BadConfig(err))
            }
            (Ok(mut config), Err(_)) | (Err(_), Ok(mut config)) => {
                let keys = Self::merge_config_keys(keymap::default(), config.keys, None);

                let (theme_lang, keys_lang, editor_lang) = Self::get_lang_maps(
                    Self::get_lang_config_map(config.languages.take()),
                    HashMap::new(),
                    &keys,
                    &config.editor,
                )?;

                let editor = Self::map_editor_config(config.editor)?;

                Config {
                    theme: config.theme,
                    keys,
                    editor,
                    theme_lang,
                    keys_lang,
                    editor_lang,
                }
            }

            // these are just two io errors return the one for the global config
            (Err(err), Err(_)) => return Err(err),
        };

        Ok(res)
    }

    fn get_lang_config_map(
        languages: Option<Vec<LanguageConfigRaw>>,
    ) -> HashMap<String, LanguageConfigRaw> {
        languages.map_or_else(
            || HashMap::new(),
            |languages| {
                languages
                    .into_iter()
                    .map(|lang| (lang.name.clone(), lang))
                    .collect()
            },
        )
    }

    fn get_lang_maps(
        mut lang_global: HashMap<String, LanguageConfigRaw>,
        mut lang_local: HashMap<String, LanguageConfigRaw>,
        merged_keys: &HashMap<Mode, KeyTrie>,
        editor_value: &Option<toml::Value>,
    ) -> Result<
        (
            HashMap<String, String>,
            HashMap<String, HashMap<Mode, KeyTrie>>,
            HashMap<String, helix_view::editor::Config>,
        ),
        ConfigLoadError,
    > {
        let mut theme_lang = HashMap::new();
        let mut keys_lang = HashMap::new();
        let mut editor_lang = HashMap::new();

        let language_names: HashSet<String> = lang_global
            .keys()
            .chain(lang_local.keys())
            .cloned()
            .collect();

        for lang in language_names {
            let (mut theme, mut keys, mut editor) =
                match (lang_global.get_mut(&lang), lang_local.get_mut(&lang)) {
                    (None, Some(lang_conf)) | (Some(lang_conf), None) => {
                        let keys = lang_conf
                            .keys
                            .take()
                            .map(|k| Self::merge_config_keys(merged_keys.clone(), Some(k), None));

                        let editor = if lang_conf.editor.is_some() {
                            Some(Self::map_editor_config(Self::merge_editor_toml(
                                editor_value.clone(),
                                lang_conf.editor.take(),
                            ))?)
                        } else {
                            None
                        };

                        (lang_conf.theme.take(), keys, editor)
                    }
                    (Some(lang_global), Some(lang_local)) => {
                        let keys = if lang_global.keys.is_some() || lang_local.keys.is_some() {
                            Some(Self::merge_config_keys(
                                merged_keys.clone(),
                                lang_global.keys.take(),
                                lang_local.keys.take(),
                            ))
                        } else {
                            None
                        };

                        let editor = if lang_global.editor.is_some() || lang_local.editor.is_some()
                        {
                            Some(Self::map_editor_config(Self::merge_editor_toml(
                                editor_value.clone(),
                                Self::merge_editor_toml(
                                    lang_global.editor.take(),
                                    lang_local.editor.take(),
                                ),
                            ))?)
                        } else {
                            None
                        };

                        (
                            lang_local.theme.take().or(lang_global.theme.take()),
                            keys,
                            editor,
                        )
                    }
                    (..) => (None, None, None),
                };

            if let Some(theme) = theme.take() {
                theme_lang.insert(lang.clone(), theme);
            }

            if let Some(keys) = keys.take() {
                keys_lang.insert(lang.clone(), keys);
            }

            if let Some(editor) = editor.take() {
                editor_lang.insert(lang, editor);
            }
        }

        Ok((theme_lang, keys_lang, editor_lang))
    }

    fn merge_config_keys(
        mut dst: HashMap<Mode, KeyTrie>,
        global_keys: Option<HashMap<Mode, KeyTrie>>,
        local_keys: Option<HashMap<Mode, KeyTrie>>,
    ) -> HashMap<Mode, KeyTrie> {
        if let Some(global_keys) = global_keys {
            merge_keys(&mut dst, global_keys)
        }
        if let Some(local_keys) = local_keys {
            merge_keys(&mut dst, local_keys)
        }

        dst
    }

    fn merge_editor_toml(
        global_editor: Option<toml::Value>,
        local_editor: Option<toml::Value>,
    ) -> Option<toml::Value> {
        match (global_editor, local_editor) {
            (None, None) => None,
            (None, Some(val)) | (Some(val), None) => Some(val),
            (Some(global), Some(local)) => Some(merge_toml_values(global, local, 3)),
        }
    }

    fn map_editor_config(
        editor_value: Option<toml::Value>,
    ) -> Result<helix_view::editor::Config, ConfigLoadError> {
        let editor = match editor_value {
            None => helix_view::editor::Config::default(),
            Some(val) => val.try_into().map_err(ConfigLoadError::BadConfig)?,
        };

        Ok(editor)
    }

    pub fn load_default() -> Result<Config, ConfigLoadError> {
        let global_config =
            fs::read_to_string(helix_loader::config_file()).map_err(ConfigLoadError::Error);
        let local_config = fs::read_to_string(helix_loader::workspace_config_file())
            .map_err(ConfigLoadError::Error);
        Config::load(global_config, local_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Config {
        fn load_test(config: &str) -> Config {
            Config::load(Ok(config.to_owned()), Err(ConfigLoadError::default())).unwrap()
        }
    }

    #[test]
    fn parsing_keymaps_config_file() {
        use crate::keymap;
        use helix_core::hashmap;
        use helix_view::document::Mode;

        let sample_keymaps = r#"
            [keys.insert]
            y = "move_line_down"
            S-C-a = "delete_selection"

            [keys.normal]
            A-F12 = "move_next_word_end"
        "#;

        let mut keys = keymap::default();
        merge_keys(
            &mut keys,
            hashmap! {
                Mode::Insert => keymap!({ "Insert mode"
                    "y" => move_line_down,
                    "S-C-a" => delete_selection,
                }),
                Mode::Normal => keymap!({ "Normal mode"
                    "A-F12" => move_next_word_end,
                }),
            },
        );

        assert_eq!(
            Config::load_test(sample_keymaps),
            Config {
                keys,
                ..Default::default()
            }
        );
    }

    #[test]
    fn keys_resolve_to_correct_defaults() {
        // From serde default
        let default_keys = Config::load_test("").keys;
        assert_eq!(default_keys, keymap::default());

        // From the Default trait
        let default_keys = Config::default().keys;
        assert_eq!(default_keys, keymap::default());
    }
}
