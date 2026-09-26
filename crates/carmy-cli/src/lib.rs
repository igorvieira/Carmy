//! Generators behind `carmy new` and `carmy generate tool`.
mod tool;
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};
pub use tool::{find_project, generate_tool, validate_tool_name};

/// Where the generated project gets the `carmy` crate from.
#[derive(Debug, Clone)]
pub enum Dependency {
    /// The crates.io release matching this CLI's minor version (default).
    CratesIo,
    /// The `main` branch of the public repository.
    Git,
    /// A local checkout: the repository root or `crates/carmy`.
    Path(PathBuf),
}
impl Dependency {
    fn toml(&self) -> io::Result<String> {
        match self {
            Self::CratesIo => Ok(format!("{:?}", release_requirement())),
            Self::Git => Ok(r#"{ git = "https://github.com/igorvieira/Carmy" }"#.into()),
            Self::Path(path) => {
                let path = fs::canonicalize(path)?;
                let facade = if path.join("crates/carmy/Cargo.toml").is_file() {
                    path.join("crates/carmy")
                } else if path.join("Cargo.toml").is_file() && path.ends_with("carmy") {
                    path
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{} is not a Carmy checkout", path.display()),
                    ));
                };
                Ok(format!("{{ path = {:?} }}", facade.display().to_string()))
            }
        }
    }
}

/// `0.1` for CLI 0.1.x: compatible releases of the matching series.
fn release_requirement() -> String {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    format!("{major}.{minor}")
}

#[derive(Debug)]
pub enum Error {
    InvalidName(String),
    InvalidEffect(String),
    Exists(PathBuf),
    NotAProject(PathBuf),
    Io(io::Error),
}
/// The former name of [`Error`].
pub type NewError = Error;
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(reason) => write!(f, "invalid name: {reason}"),
            Self::InvalidEffect(effect) => write!(
                f,
                "invalid effect `{effect}`; use none, read, write, external_write or destructive"
            ),
            Self::Exists(path) => write!(f, "{} already exists; not overwriting", path.display()),
            Self::NotAProject(dir) => write!(
                f,
                "{} is not inside a Carmy application (no src/tools/mod.rs found); run `carmy new` first",
                dir.display()
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A package name Cargo accepts and that does not shadow Carmy or the standard library.
pub fn validate_name(name: &str) -> Result<(), Error> {
    let invalid = |reason: &str| Err(Error::InvalidName(format!("`{name}` {reason}")));
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return invalid("must start with a lowercase ASCII letter");
    }
    if name.len() > 64 {
        return invalid("must be at most 64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return invalid("may only contain a-z, 0-9, `_` and `-`");
    }
    if ["carmy", "core", "std", "alloc", "test", "proc_macro"].contains(&name) {
        return invalid("is reserved");
    }
    Ok(())
}

const FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../templates/Cargo.toml.tpl")),
    ("carmy.toml", include_str!("../templates/carmy.toml.tpl")),
    ("README.md", include_str!("../templates/README.md.tpl")),
    (".gitignore", include_str!("../templates/gitignore.tpl")),
    ("src/main.rs", include_str!("../templates/main.rs.tpl")),
    (
        "src/tools/mod.rs",
        include_str!("../templates/tools_mod.rs.tpl"),
    ),
    (
        "src/tools/hello.rs",
        include_str!("../templates/hello.rs.tpl"),
    ),
];

/// Create `<parent>/<name>` with the conventional layout. Returns the project directory.
pub fn generate(parent: &Path, name: &str, carmy: &Dependency) -> Result<PathBuf, Error> {
    validate_name(name)?;
    let root = parent.join(name);
    if root.exists() {
        return Err(Error::Exists(root));
    }
    let dependency = carmy.toml()?;
    for (path, template) in FILES {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("files live in the project"))?;
        let contents = template
            .replace("{{name}}", name)
            .replace("{{carmy}}", &dependency);
        fs::write(path, contents)?;
    }
    Ok(root)
}
