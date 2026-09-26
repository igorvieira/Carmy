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
    /// `[http]` table: connection limits and browser protections.
    #[serde(default)]
    pub http: HttpConfig,
    /// `[jobs]` table: the worker.
    #[serde(default)]
    pub jobs: JobsConfig,
}

/// ```toml
/// [jobs]
/// concurrency = 4      # CARMY_JOBS_CONCURRENCY
/// max_attempts = 5     # CARMY_JOBS_MAX_ATTEMPTS
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobsConfig {
    pub concurrency: Option<usize>,
    pub max_attempts: Option<u32>,
}

/// ```toml
/// [http]
/// header_timeout_secs = 10    # CARMY_HTTP_HEADER_TIMEOUT_SECS
/// body_timeout_secs = 30      # CARMY_HTTP_BODY_TIMEOUT_SECS
/// max_connections = 4096      # CARMY_HTTP_MAX_CONNECTIONS
/// security_headers = false    # CARMY_HTTP_SECURITY_HEADERS
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    pub header_timeout_secs: Option<u64>,
    pub body_timeout_secs: Option<u64>,
    pub max_connections: Option<usize>,
    pub security_headers: Option<bool>,
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
        fn number<T: std::str::FromStr>(key: &str, value: String) -> Result<T, String> {
            value
                .parse()
                .map_err(|_| format!("{key} must be a number, got `{value}`"))
        }
        if let Some(seconds) = env("CARMY_TIMEOUT_SECS") {
            config.timeout_secs = Some(number("CARMY_TIMEOUT_SECS", seconds)?);
        }
        if let Some(seconds) = env("CARMY_HTTP_HEADER_TIMEOUT_SECS") {
            config.http.header_timeout_secs =
                Some(number("CARMY_HTTP_HEADER_TIMEOUT_SECS", seconds)?);
        }
        if let Some(seconds) = env("CARMY_HTTP_BODY_TIMEOUT_SECS") {
            config.http.body_timeout_secs = Some(number("CARMY_HTTP_BODY_TIMEOUT_SECS", seconds)?);
        }
        if let Some(max) = env("CARMY_HTTP_MAX_CONNECTIONS") {
            config.http.max_connections = Some(number("CARMY_HTTP_MAX_CONNECTIONS", max)?);
        }
        if let Some(n) = env("CARMY_JOBS_CONCURRENCY") {
            config.jobs.concurrency = Some(number("CARMY_JOBS_CONCURRENCY", n)?);
        }
        if let Some(n) = env("CARMY_JOBS_MAX_ATTEMPTS") {
            config.jobs.max_attempts = Some(number("CARMY_JOBS_MAX_ATTEMPTS", n)?);
        }
        if let Some(enabled) = env("CARMY_HTTP_SECURITY_HEADERS") {
            config.http.security_headers = Some(match enabled.as_str() {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                other => {
                    return Err(format!(
                        "CARMY_HTTP_SECURITY_HEADERS must be true or false, got `{other}`"
                    ));
                }
            });
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
    fn http_table_and_env_overrides() {
        let file = "[http]\nheader_timeout_secs = 5\nsecurity_headers = true";
        let env = |key: &str| (key == "CARMY_HTTP_MAX_CONNECTIONS").then(|| "64".to_string());
        let config = Config::from_sources(Some(file), env).unwrap();
        assert_eq!(config.http.header_timeout_secs, Some(5));
        assert_eq!(config.http.security_headers, Some(true));
        assert_eq!(config.http.max_connections, Some(64));
        assert!(Config::from_sources(Some("[http]\nheader_timeout = 5"), |_| None).is_err());
        let env = |key: &str| (key == "CARMY_HTTP_SECURITY_HEADERS").then(|| "maybe".to_string());
        assert!(
            Config::from_sources(None, env)
                .unwrap_err()
                .contains("true or false")
        );
    }
    #[test]
    fn jobs_table_and_env_overrides() {
        let config = Config::from_sources(Some("[jobs]\nconcurrency = 8"), |key| {
            (key == "CARMY_JOBS_MAX_ATTEMPTS").then(|| "3".to_string())
        })
        .unwrap();
        assert_eq!(config.jobs.concurrency, Some(8));
        assert_eq!(config.jobs.max_attempts, Some(3));
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
