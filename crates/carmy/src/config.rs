//! Application configuration: `carmy.toml`, overridden by `CARMY_*` environment variables.
use crate::Error;
use serde::Deserialize;

/// Settings read by [`carmy::app()`](crate::app). Every field is optional.
///
/// ```toml
/// name = "shop"               # CARMY_NAME
/// address = "127.0.0.1:3000"  # CARMY_ADDR
/// timeout_secs = 30           # CARMY_TIMEOUT_SECS
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub name: Option<String>,
    pub address: Option<String>,
    pub timeout_secs: Option<u64>,
}
impl Config {
    /// Read `carmy.toml` from the working directory (or the file named by
    /// `CARMY_CONFIG`), then apply environment overrides.
    pub fn load() -> Result<Self, Error> {
        let path = std::env::var("CARMY_CONFIG").ok();
        let explicit = path.is_some();
        let path = path.unwrap_or_else(|| "carmy.toml".into());
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => None,
            Err(e) => return Err(Error::Config(format!("{path}: {e}"))),
        };
        Self::from_sources(file.as_deref(), |key| std::env::var(key).ok())
            .map_err(|e| Error::Config(format!("{path}: {e}")))
    }
    /// Precedence: environment, then file, then defaults.
    pub fn from_sources(
        file: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut config: Self = match file {
            Some(text) => toml::from_str(text).map_err(|e| e.message().to_owned())?,
            None => Self::default(),
        };
        if let Some(name) = env("CARMY_NAME") {
            config.name = Some(name);
        }
        if let Some(address) = env("CARMY_ADDR") {
            config.address = Some(address);
        }
        if let Some(seconds) = env("CARMY_TIMEOUT_SECS") {
            let seconds = seconds
                .parse()
                .map_err(|_| format!("CARMY_TIMEOUT_SECS must be a number, got `{seconds}`"))?;
            config.timeout_secs = Some(seconds);
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    #[test]
    fn environment_overrides_file() {
        let file = "name = \"shop\"\naddress = \"0.0.0.0:80\"\ntimeout_secs = 5";
        let env = |key: &str| (key == "CARMY_ADDR").then(|| "127.0.0.1:9000".to_string());
        let config = Config::from_sources(Some(file), env).unwrap();
        assert_eq!(config.name.as_deref(), Some("shop"));
        assert_eq!(config.address.as_deref(), Some("127.0.0.1:9000"));
        assert_eq!(config.timeout_secs, Some(5));
        assert_eq!(
            Config::from_sources(None, |_| None).unwrap(),
            Config::default()
        );
    }
    #[test]
    fn typos_and_bad_values_are_reported() {
        let typo = Config::from_sources(Some("adress = \"x\""), |_| None).unwrap_err();
        assert!(typo.contains("adress"), "{typo}");
        let env = |key: &str| (key == "CARMY_TIMEOUT_SECS").then(|| "soon".to_string());
        assert!(
            Config::from_sources(None, env)
                .unwrap_err()
                .contains("soon")
        );
    }
}
