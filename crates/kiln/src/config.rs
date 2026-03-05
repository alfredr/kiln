use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub site: SiteConfig,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub hints: Vec<HintConfig>,
}

#[derive(Debug, Deserialize)]
pub struct SiteConfig {
    pub title: String,
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_author")]
    pub author: String,
}

#[derive(Debug, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_content_dir")]
    pub content_dir: PathBuf,
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,
    #[serde(default = "default_template_dir")]
    pub template_dir: PathBuf,
    #[serde(default = "default_static_dir")]
    pub static_dir: PathBuf,
    #[serde(default)]
    pub slug_pattern: Option<String>,
    /// Path to a custom `.kiln` prelude file. Omit to use the built-in prelude.
    #[serde(default)]
    pub prelude: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HintConfig {
    /// CSS-like selector string (e.g. `"code-block[lang=\"rust\"]"`).
    pub selector: Option<String>,
    /// Legacy: match by kind name (PascalCase).
    pub kind: Option<String>,
    /// Legacy: match by CSS class.
    pub class: Option<String>,
    /// Legacy: match by explicit ID.
    pub id: Option<String>,
    #[serde(flatten)]
    pub properties: HashMap<String, toml::Value>,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            content_dir: default_content_dir(),
            output_dir: default_output_dir(),
            template_dir: default_template_dir(),
            static_dir: default_static_dir(),
            slug_pattern: None,
            prelude: None,
        }
    }
}

fn default_url() -> String {
    "http://localhost:3000".into()
}
fn default_author() -> String {
    String::new()
}
fn default_content_dir() -> PathBuf {
    "content".into()
}
fn default_output_dir() -> PathBuf {
    "dist".into()
}
fn default_template_dir() -> PathBuf {
    "templates".into()
}
fn default_static_dir() -> PathBuf {
    "static".into()
}

impl Config {
    pub fn load(project_root: &Path) -> anyhow::Result<Self> {
        let path = project_root.join("kiln.toml");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Config = toml::from_str(&text).with_context(|| "failed to parse kiln.toml")?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_valid_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("kiln.toml"),
            "[site]\ntitle = \"Test Site\"\nurl = \"https://example.com\"\nauthor = \"Alice\"\n",
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        assert_eq!(config.site.title, "Test Site");
        assert_eq!(config.site.url, "https://example.com");
        assert_eq!(config.site.author, "Alice");
    }

    #[test]
    fn load_minimal_config_applies_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("kiln.toml"),
            "[site]\ntitle = \"Minimal\"\n",
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        assert_eq!(config.site.title, "Minimal");
        assert_eq!(config.site.url, "http://localhost:3000");
        assert_eq!(config.site.author, "");
        assert_eq!(config.build.content_dir, PathBuf::from("content"));
        assert_eq!(config.build.output_dir, PathBuf::from("dist"));
        assert!(config.build.slug_pattern.is_none());
    }

    #[test]
    fn load_hints_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("kiln.toml"),
            "[site]\ntitle = \"Test\"\n\n[[hints]]\nkind = \"CodeBlock\"\npreview = true\n",
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        assert_eq!(config.hints.len(), 1);
        assert_eq!(config.hints[0].kind.as_deref(), Some("CodeBlock"));
        assert_eq!(
            config.hints[0].properties.get("preview"),
            Some(&toml::Value::Boolean(true))
        );
    }

    #[test]
    fn load_missing_file_errors() {
        let result = Config::load(Path::new("/nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn load_malformed_toml_errors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("kiln.toml"), "not valid { toml").unwrap();

        let result = Config::load(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn load_config_with_prelude() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("kiln.toml"),
            "[site]\ntitle = \"Test\"\n\n[build]\nprelude = \"custom.kiln\"\n",
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        assert_eq!(config.build.prelude.as_deref(), Some("custom.kiln"));
    }

    #[test]
    fn load_config_without_prelude() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("kiln.toml"),
            "[site]\ntitle = \"Test\"\n",
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        assert!(config.build.prelude.is_none());
    }
}
