use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserKind {
    #[default]
    Orion,
    Chromium,
    Chrome,
    Brave,
    Arc,
    Edge,
    Firefox,
    Safari,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    #[default]
    LmStudio,
    Openai,
    Anthropic,
    #[serde(rename = "none")]
    Disabled,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn expand_tilde(p: PathBuf) -> PathBuf {
    let s = match p.to_str() {
        Some(s) => s,
        None => return p,
    };
    if s == "~" {
        return home_dir();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    p
}

fn deserialize_expanded_path<'de, D>(d: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    PathBuf::deserialize(d).map(expand_tilde)
}

fn serialize_path<S: serde::Serializer>(path: &Path, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&path.to_string_lossy())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmbedModel {
    /// English — 384-dim, ~130 MB (default)
    #[default]
    #[serde(rename = "bge_small_en_v1_5")]
    BgeSmallEnV15,
    /// English — 768-dim, ~430 MB, higher accuracy
    #[serde(rename = "bge_base_en_v1_5")]
    BgeBaseEnV15,
    /// English — 768-dim, ~270 MB, strong accuracy
    #[serde(rename = "nomic_embed_text_v1_5")]
    NomicEmbedTextV15,
    /// English — 384-dim, ~90 MB, fastest/smallest
    AllMiniLmL6V2,
    /// Multilingual (100+ languages) — 384-dim, ~120 MB
    MultilingualE5Small,
    /// Multilingual (100+ languages) — 768-dim, ~280 MB, better accuracy
    MultilingualE5Base,
    /// Multilingual (50+ languages) — 384-dim, ~120 MB
    ParaphraseMultilingualMiniLmL12V2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedSettings {
    pub enabled: bool,
    pub model: EmbedModel,
    pub vector_search: bool,
}

impl Default for EmbedSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: EmbedModel::default(),
            vector_search: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub browser: BrowserSettings,
    pub data: DataSettings,
    pub fetch: FetchSettings,
    pub llm: LlmSettings,
    pub sync: SyncSettings,
    pub embed: EmbedSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApplicationSettings {
    pub host: String,
    pub port: u16,
    pub ui_poll_secs: u64,
    pub custom_css: String,
    pub hotkey: String,
    pub cluster_score_high: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserSettings {
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_expanded_path"
    )]
    pub history_db_path: PathBuf,
    pub kind: BrowserKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DataSettings {
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_expanded_path"
    )]
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FetchSettings {
    pub delay_ms: u64,
    pub timeout_secs: u64,
    pub user_agent: String,
    pub ban: Vec<String>,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    pub provider: LlmProvider,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    /// Total character budget for the context block sent to the LLM.
    /// Divided evenly across retrieved sources. Default: 8000 (~2k tokens).
    pub max_context_chars: usize,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSettings {
    pub interval_mins: u64,
    pub fetch_batch: u32,
    pub embed_batch: u32,
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8734,
            ui_poll_secs: 30,
            custom_css: String::new(),
            hotkey: "CmdOrCtrl+Shift+Space".to_string(),
            cluster_score_high: 0.65,
        }
    }
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            history_db_path: home_dir().join("Library/Application Support/Orion/Defaults/history"),
            kind: BrowserKind::Orion,
        }
    }
}

impl Default for DataSettings {
    fn default() -> Self {
        Self {
            dir: home_dir().join(".memoir"),
        }
    }
}

impl Default for FetchSettings {
    fn default() -> Self {
        Self {
            delay_ms: 200,
            timeout_secs: 15,
            user_agent: "memoir/0.1 (personal history indexer)".to_string(),
            ban: DEFAULT_BAN_LIST.iter().map(|s| s.to_string()).collect(),
            max_retries: 3,
        }
    }
}

/// Default hosts/prefixes that are almost always private, login-gated, or
/// produce no useful indexable content. Users can remove entries they want
/// indexed, or add their own in Settings → Fetch → Ban list.
const DEFAULT_BAN_LIST: &[&str] = &[
    // ----- email -----
    "mail.google.com",
    "mail.yahoo.com",
    "outlook.live.com",
    "outlook.office.com",
    "mail.proton.me",
    "app.fastmail.com",
    // ----- auth / account management -----
    "accounts.google.com",
    "myaccount.google.com",
    "login.microsoftonline.com",
    "login.live.com",
    "appleid.apple.com",
    // ----- social (login walls) -----
    "facebook.com",
    "instagram.com",
    "linkedin.com",
    // ----- messaging -----
    "web.whatsapp.com",
    "web.telegram.org",
    "discord.com",
    // ----- cloud storage / docs -----
    "drive.google.com",
    "docs.google.com",
    "dropbox.com",
    "onedrive.live.com",
    // ----- streaming (login-gated content) -----
    "netflix.com",
    "hulu.com",
    "disneyplus.com",
    "primevideo.com",
    "max.com",
    "peacocktv.com",
    "paramountplus.com",
];

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            provider: LlmProvider::LmStudio,
            base_url: "http://localhost:1234".to_string(),
            model: "local-model".to_string(),
            api_key: None,
            max_context_chars: 8_000,
            system_prompt: None,
        }
    }
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            interval_mins: 60,
            fetch_batch: 500,
            embed_batch: 200,
        }
    }
}

impl FetchSettings {
    pub fn is_banned(&self, url: &str) -> bool {
        self.ban.iter().any(|p| matches_ban_pattern(url, p))
    }
}

/// Returns true if `url` matches the ban pattern `p`.
///
/// - No `/` in `p` → host match: exact host or any subdomain of `p`.
/// - `/` in `p` → path-prefix match: URL must start with `http(s)://p`
///   and be followed by `/`, `?`, `#`, or end-of-string so partial path
///   segments (e.g. `mycompany-public`) are not accidentally caught.
pub fn matches_ban_pattern(url: &str, p: &str) -> bool {
    if p.contains('/') {
        for scheme in ["https://", "http://"] {
            let base = format!("{scheme}{p}");
            if url == base
                || url.starts_with(&format!("{base}/"))
                || url.starts_with(&format!("{base}?"))
                || url.starts_with(&format!("{base}#"))
            {
                return true;
            }
        }
        false
    } else {
        let host = host_from_url(url);
        host == p || (!is_ip(host) && host.ends_with(&format!(".{p}")))
    }
}

impl Settings {
    pub fn config_dir() -> std::path::PathBuf {
        std::env::var_os("MEMOIR_CONFIG_DIR")
            .map(std::path::PathBuf::from)
            .map(expand_tilde)
            .unwrap_or_else(|| home_dir().join(".memoir"))
    }

    /// Validate settings and return errors for any invalid configuration.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.sync.interval_mins == 0 {
            errors.push("sync interval cannot be zero".to_string());
        }

        if self.fetch.timeout_secs == 0 {
            errors.push("fetch timeout cannot be zero".to_string());
        }

        if self.fetch.max_retries == 0 {
            errors.push("fetch max_retries cannot be zero".to_string());
        }

        if self.application.port == 0 {
            errors.push("application port cannot be zero".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn load() -> Self {
        Self::load_from(&Self::config_dir())
    }

    pub fn load_from(config_dir: &std::path::Path) -> Self {
        let path = config_dir.join("config.toml");
        if !path.exists() {
            return Self::default();
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("memoir: failed to read config ({path:?}): {e}");
                return Self::default();
            }
        };
        let settings: Self = match toml::from_str(&text) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("memoir: invalid config ({path:?}): {e}");
                return Self::default();
            }
        };

        if let Err(errors) = settings.validate() {
            for err in errors {
                eprintln!("memoir config warning: {}", err);
            }
        }

        settings
    }
}

fn is_ip(host: &str) -> bool {
    // IPv6 literal enclosed in brackets, or IPv4 (all digits and dots).
    host.starts_with('[') || host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

pub fn host_from_url(url: &str) -> &str {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..end];
    match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banned(entries: &[&str]) -> FetchSettings {
        FetchSettings {
            ban: entries.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    // --- Ban pattern tests ---

    #[test]
    fn exact_domain_is_banned() {
        let f = banned(&["gmail.com"]);
        assert!(f.is_banned("https://gmail.com/mail/u/0/"));
    }

    #[test]
    fn subdomain_is_banned() {
        let f = banned(&["gmail.com"]);
        assert!(f.is_banned("https://m.gmail.com/"));
    }

    #[test]
    fn unrelated_domain_is_not_banned() {
        let f = banned(&["gmail.com"]);
        assert!(!f.is_banned("https://notgmail.com/"));
    }

    #[test]
    fn empty_ban_list_never_bans() {
        let f = FetchSettings::default();
        assert!(!f.is_banned("https://gmail.com/"));
    }

    #[test]
    fn port_is_stripped_before_matching() {
        let f = banned(&["example.com"]);
        assert!(f.is_banned("http://example.com:8080/path"));
    }

    #[test]
    fn deep_subdomain_is_banned() {
        let f = banned(&["google.com"]);
        assert!(f.is_banned("https://mail.accounts.google.com/"));
    }

    #[test]
    fn ip_address_exact_match() {
        let f = banned(&["127.0.0.1"]);
        assert!(f.is_banned("http://127.0.0.1/"));
        assert!(f.is_banned("http://127.0.0.1/some/path?q=1"));
    }

    #[test]
    fn ip_address_port_stripped() {
        let f = banned(&["127.0.0.1"]);
        assert!(f.is_banned("http://127.0.0.1:8080/"));
    }

    #[test]
    fn ip_address_no_spurious_suffix_match() {
        // "0.1" must NOT match 127.0.0.1 via the subdomain ends_with logic.
        let f = banned(&["0.1"]);
        assert!(!f.is_banned("http://127.0.0.1/"));
    }

    #[test]
    fn different_ip_not_banned() {
        let f = banned(&["127.0.0.1"]);
        assert!(!f.is_banned("http://192.168.1.1/"));
    }

    // --- path-prefix bans ---

    #[test]
    fn path_prefix_blocks_matching_org() {
        let f = banned(&["github.com/mycompany"]);
        assert!(f.is_banned("https://github.com/mycompany/private-repo"));
        assert!(f.is_banned("https://github.com/mycompany/private-repo/issues/1"));
    }

    #[test]
    fn path_prefix_does_not_block_sibling_org() {
        let f = banned(&["github.com/mycompany"]);
        assert!(!f.is_banned("https://github.com/mycompany-public/repo"));
        assert!(!f.is_banned("https://github.com/otherorg/repo"));
    }

    #[test]
    fn path_prefix_blocks_exact_url() {
        let f = banned(&["github.com/mycompany"]);
        assert!(f.is_banned("https://github.com/mycompany"));
    }

    #[test]
    fn path_prefix_blocks_with_query_string() {
        let f = banned(&["github.com/mycompany"]);
        assert!(f.is_banned("https://github.com/mycompany?tab=repositories"));
    }

    #[test]
    fn path_prefix_allows_rest_of_host() {
        let f = banned(&["github.com/mycompany"]);
        assert!(!f.is_banned("https://github.com/"));
        assert!(!f.is_banned("https://github.com/torvalds/linux"));
    }

    // --- host_from_url tests ---

    #[test]
    fn host_from_url_https() {
        assert_eq!(host_from_url("https://example.com/path"), "example.com");
    }

    #[test]
    fn host_from_url_http() {
        assert_eq!(host_from_url("http://example.com/path"), "example.com");
    }

    #[test]
    fn host_from_url_with_port() {
        assert_eq!(host_from_url("http://example.com:8080/path"), "example.com");
    }

    #[test]
    fn host_from_url_subdomain() {
        assert_eq!(
            host_from_url("https://sub.example.com/path"),
            "sub.example.com"
        );
    }

    #[test]
    fn host_from_url_query_string() {
        assert_eq!(host_from_url("https://example.com?q=test"), "example.com");
    }

    #[test]
    fn host_from_url_fragment() {
        assert_eq!(host_from_url("https://example.com#section"), "example.com");
    }

    // --- Settings validation tests ---

    #[test]
    fn valid_settings_pass_validation() {
        let settings = Settings::default();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn zero_sync_interval_fails_validation() {
        let mut settings = Settings::default();
        settings.sync.interval_mins = 0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn zero_fetch_timeout_fails_validation() {
        let mut settings = Settings::default();
        settings.fetch.timeout_secs = 0;
        let result = settings.validate();
        assert!(result.is_err());
        if let Err(errors) = result {
            assert!(errors.iter().any(|e| e.contains("fetch timeout")));
        }
    }

    #[test]
    fn zero_max_retries_fails_validation() {
        let mut settings = Settings::default();
        settings.fetch.max_retries = 0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn zero_port_fails_validation() {
        let mut settings = Settings::default();
        settings.application.port = 0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn multiple_validation_errors_returned() {
        let mut settings = Settings::default();
        settings.sync.interval_mins = 0;
        settings.fetch.timeout_secs = 0;
        let result = settings.validate();
        assert!(result.is_err());
        if let Err(errors) = result {
            assert!(errors.len() >= 2);
        }
    }
}
