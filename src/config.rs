//! Configuration: profiles, validation, the built-in `hectorir` preset, and shareable profile packs.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_DESTINATION: &str = "/Volumes/Old External Drives/0 Photos & My Footage/Needs to Back up";
pub const MAX_CONFIG_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Base folder; each run creates `<destination>/<date>/<subfolder>/`.
    #[serde(default = "default_destination")]
    pub destination: String,
    /// chrono/strftime format for the dated folder.
    #[serde(default = "default_date_format")]
    pub date_format: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Send a desktop notification when a copy finishes.
    #[serde(default = "yes")]
    pub notify: bool,
    /// Default for each run: "copy" keeps originals on the card; "move" deletes each original once its copy is verified.
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default, rename = "profile")]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    /// Subfolder inside the dated folder ("" = no subfolder).
    #[serde(default)]
    pub subfolder: String,
    /// Directories to scan (recursively).
    pub sources: Vec<String>,
    /// Extensions to copy (case-insensitive, without the dot).
    pub include: Vec<String>,
    /// Extensions to never copy, even if also included.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Banner art: "action", "mirrorless" or "camera".
    #[serde(default = "default_icon")]
    pub icon: String,
    /// Optional per-profile destination override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

fn default_destination() -> String {
    DEFAULT_DESTINATION.into()
}
fn default_date_format() -> String {
    "%Y-%m-%d".into()
}
fn default_theme() -> String {
    "catppuccin".into()
}
fn default_icon() -> String {
    "camera".into()
}
fn default_mode() -> String {
    "copy".into()
}
fn yes() -> bool {
    true
}

impl Default for Config {
    /// The `hectorir` preset.
    fn default() -> Self {
        Config {
            destination: default_destination(),
            date_format: default_date_format(),
            theme: default_theme(),
            notify: true,
            mode: default_mode(),
            profiles: vec![
                Profile {
                    name: "DJI Osmo Action".into(),
                    subfolder: "DJI".into(),
                    sources: vec!["/Volumes/SD_Card/DCIM/DJI_001".into(), "/Volumes/OsmoAction/DCIM".into()],
                    include: vec!["mp4".into(), "wav".into()],
                    exclude: vec!["lrf".into()],
                    icon: "action".into(),
                    destination: None,
                },
                Profile {
                    name: "Sony Mirrorless".into(),
                    subfolder: "Sony".into(),
                    sources: vec!["/Volumes/Untitled/DCIM".into(), "/Volumes/Untitled/private/M4ROOT/CLIP".into()],
                    include: vec!["mp4".into(), "arw".into()],
                    exclude: vec![],
                    icon: "mirrorless".into(),
                    destination: None,
                },
            ],
        }
    }
}

/// Normalize an extension: trim, strip a leading dot, lowercase.
pub fn norm_ext(e: &str) -> String {
    e.trim().trim_start_matches('.').to_lowercase()
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(h) = dirs::home_dir()
    {
        return h.join(rest);
    }
    PathBuf::from(p)
}

impl Profile {
    pub fn source_paths(&self) -> Vec<PathBuf> {
        self.sources.iter().map(|s| expand_tilde(s)).collect()
    }

    /// Does `ext` (any case) pass this profile's include/exclude lists?
    pub fn wants_ext(&self, ext: &str) -> bool {
        let e = norm_ext(ext);
        !self.exclude.iter().any(|x| norm_ext(x) == e) && self.include.iter().any(|x| norm_ext(x) == e)
    }

    /// Reject anything that could escape the dated folder or is nonsensical.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("profile name is empty");
        }
        if self.name.chars().any(|c| c.is_control()) {
            bail!("profile '{}': name contains control characters", self.name);
        }
        validate_component(&self.subfolder).with_context(|| format!("profile '{}': invalid subfolder", self.name))?;
        if self.sources.is_empty() || self.sources.iter().any(|s| s.trim().is_empty()) {
            bail!("profile '{}': needs at least one non-empty source", self.name);
        }
        if self.include.is_empty() {
            bail!("profile '{}': 'include' must list at least one extension", self.name);
        }
        for e in self.include.iter().chain(self.exclude.iter()) {
            let n = norm_ext(e);
            if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric()) {
                bail!("profile '{}': bad extension '{}'", self.name, e);
            }
        }
        if let Some(d) = &self.destination
            && d.trim().is_empty()
        {
            bail!("profile '{}': empty destination override", self.name);
        }
        Ok(())
    }
}

/// A subfolder/date component must be a single, plain path segment.
pub fn validate_component(s: &str) -> Result<()> {
    if s.is_empty() {
        return Ok(());
    }
    if s == "." || s == ".." || s.contains('/') || s.contains('\\') || s.contains('\0') || s.chars().any(|c| c.is_control()) {
        bail!("'{}' must be a single plain folder name", s);
    }
    Ok(())
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.destination.trim().is_empty() {
            bail!("destination is empty");
        }
        if !matches!(self.mode.to_lowercase().as_str(), "copy" | "move") {
            bail!("mode must be \"copy\" or \"move\" (got \"{}\")", self.mode);
        }
        if self.profiles.is_empty() {
            bail!("no profiles defined");
        }
        // The date format must render to a single safe folder name.
        let rendered = self.render_date(chrono::Local::now())?;
        validate_component(&rendered).context("date_format renders an unsafe folder name")?;
        if rendered.is_empty() {
            bail!("date_format renders an empty name");
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.profiles {
            p.validate()?;
            if !seen.insert(p.name.to_lowercase()) {
                bail!("duplicate profile name '{}'", p.name);
            }
        }
        Ok(())
    }

    pub fn render_date(&self, when: chrono::DateTime<chrono::Local>) -> Result<String> {
        use std::fmt::Write;
        let mut s = String::new();
        write!(s, "{}", when.format(&self.date_format)).map_err(|_| anyhow::anyhow!("invalid date_format '{}'", self.date_format))?;
        Ok(s)
    }

    pub fn dest_base_for(&self, p: &Profile) -> PathBuf {
        expand_tilde(p.destination.as_deref().unwrap_or(&self.destination))
    }

    pub fn find_profile(&self, name: &str) -> Option<&Profile> {
        let n = name.to_lowercase();
        self.profiles
            .iter()
            .find(|p| p.name.to_lowercase() == n)
            .or_else(|| self.profiles.iter().find(|p| p.name.to_lowercase().contains(&n) || p.subfolder.to_lowercase() == n))
    }

    pub fn from_toml(text: &str) -> Result<Config> {
        let cfg: Config = toml::from_str(text).context("parsing config TOML")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Config> {
        let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
        if meta.len() > MAX_CONFIG_BYTES {
            bail!("config file is unreasonably large ({} bytes)", meta.len());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Config::from_toml(&text).with_context(|| format!("in {}", path.display()))
    }

    /// Load the file if it exists, otherwise use the built-in preset.
    pub fn load_or_default(path: &Path) -> Result<Config> {
        if path.exists() { Config::load(path) } else { Ok(Config::default()) }
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Atomically save, keeping a `.bak` of the previous file.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if path.exists() {
            let _ = std::fs::copy(path, path.with_extension("toml.bak"));
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(x).join("offload");
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".config").join("offload")
}

pub fn data_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(x).join("offload");
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".local").join("share").join("offload")
}

pub fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

// ---------- Profile packs: a single shareable profile in a TOML file ----------

#[derive(Serialize, Deserialize)]
struct Pack {
    profile: Profile,
}

pub fn export_pack(p: &Profile) -> Result<String> {
    let mut p = p.clone();
    p.destination = None; // packs describe a camera, not your drives
    let body = toml::to_string_pretty(&Pack { profile: p })?;
    Ok(format!("# offload profile pack - import with: offload pack import <file>\n{body}"))
}

pub fn parse_pack(text: &str) -> Result<Profile> {
    if text.len() as u64 > MAX_CONFIG_BYTES {
        bail!("pack file too large");
    }
    let pack: Pack = toml::from_str(text).context("parsing profile pack")?;
    pack.profile.validate()?;
    Ok(pack.profile)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid_and_matches_hectorir() {
        let c = Config::default();
        c.validate().unwrap();
        assert_eq!(c.profiles.len(), 2);
        assert_eq!(c.profiles[0].include, ["mp4", "wav"]);
        assert_eq!(c.profiles[0].exclude, ["lrf"]);
        assert_eq!(c.profiles[1].include, ["mp4", "arw"]);
        assert_eq!(c.profiles[1].sources[1], "/Volumes/Untitled/private/M4ROOT/CLIP");
    }

    #[test]
    fn toml_roundtrip() {
        let c = Config::default();
        let back = Config::from_toml(&c.to_toml().unwrap()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn extension_matching_is_case_insensitive_and_exclude_wins() {
        let p = &Config::default().profiles[0];
        assert!(p.wants_ext("MP4"));
        assert!(p.wants_ext(".wav"));
        assert!(!p.wants_ext("LRF"));
        assert!(!p.wants_ext("jpg"));
        let mut q = p.clone();
        q.include.push("lrf".into());
        assert!(!q.wants_ext("lrf"), "exclude must win over include");
    }

    #[test]
    fn rejects_path_traversal_subfolders() {
        for bad in ["..", "../x", "a/b", "/abs", "a\\b", ".", "x\ny"] {
            let mut c = Config::default();
            c.profiles[0].subfolder = bad.into();
            assert!(c.validate().is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn mode_defaults_to_copy_and_is_validated() {
        let c = Config::default();
        assert_eq!(c.mode, "copy");
        assert!(Config::from_toml("mode = 'move'\n[[profile]]\nname='X'\nsources=['/a']\ninclude=['jpg']\n").is_ok());
        assert!(Config::from_toml("mode = 'delete'\n[[profile]]\nname='X'\nsources=['/a']\ninclude=['jpg']\n").is_err());
        let c = Config::from_toml("[[profile]]\nname='X'\nsources=['/a']\ninclude=['jpg']\n").unwrap();
        assert_eq!(c.mode, "copy");
    }

    #[test]
    fn rejects_unsafe_date_format() {
        let mut c = Config::default();
        c.date_format = "%Y/%m/%d".into();
        assert!(c.validate().is_err());
        c.date_format = "%Y-%m-%d".into();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn rejects_bad_extensions_and_empty_things() {
        let mut c = Config::default();
        c.profiles[0].include = vec!["m p4".into()];
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.profiles[0].sources.clear();
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.profiles[1].name = c.profiles[0].name.to_uppercase();
        assert!(c.validate().is_err(), "duplicate names");
        let mut c = Config::default();
        c.profiles.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn minimal_config_uses_defaults() {
        let c = Config::from_toml("[[profile]]\nname='X'\nsources=['/a']\ninclude=['jpg']\n").unwrap();
        assert_eq!(c.destination, DEFAULT_DESTINATION);
        assert_eq!(c.profiles[0].icon, "camera");
    }

    #[test]
    fn find_profile_is_forgiving() {
        let c = Config::default();
        assert_eq!(c.find_profile("dji").unwrap().subfolder, "DJI");
        assert_eq!(c.find_profile("SONY").unwrap().subfolder, "Sony");
        assert!(c.find_profile("nope").is_none());
    }

    #[test]
    fn pack_roundtrip_strips_destination() {
        let mut p = Config::default().profiles[0].clone();
        p.destination = Some("/secret/drive".into());
        let text = export_pack(&p).unwrap();
        assert!(!text.contains("secret"));
        let back = parse_pack(&text).unwrap();
        p.destination = None;
        assert_eq!(back, p);
    }

    #[test]
    fn pack_import_validates() {
        assert!(parse_pack("[profile]\nname='x'\nsubfolder='../../etc'\nsources=['/a']\ninclude=['mp4']\n").is_err());
        assert!(parse_pack("garbage").is_err());
    }

    #[test]
    fn save_and_load_with_backup() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("c/config.toml");
        let mut c = Config::default();
        c.save(&path).unwrap();
        c.theme = "nord".into();
        c.save(&path).unwrap();
        assert!(d.path().join("c/config.toml.bak").exists());
        assert_eq!(Config::load(&path).unwrap().theme, "nord");
    }

    #[test]
    fn load_rejects_huge_files() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        std::fs::write(&p, vec![b'#'; (MAX_CONFIG_BYTES + 1) as usize]).unwrap();
        assert!(Config::load(&p).is_err());
    }
}
