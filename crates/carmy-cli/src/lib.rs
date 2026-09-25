//! Project generator behind `carmy new`.
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

/// Where the generated project gets the `carmy` crate from.
#[derive(Debug, Clone)]
pub enum Dependency {
    /// The public repository (default until Carmy is published on crates.io).
    Git,
    /// A local checkout: the repository root or `crates/carmy`.
    Path(PathBuf),
}
impl Dependency {
    fn toml(&self) -> io::Result<String> {
        match self {
            Self::Git => Ok(r#"{ git = "https://github.com/igorvieira/carmy" }"#.into()),
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

#[derive(Debug)]
pub enum NewError {
    InvalidName(String),
    Exists(PathBuf),
    Io(io::Error),
}
impl fmt::Display for NewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(reason) => write!(f, "invalid application name: {reason}"),
            Self::Exists(path) => write!(f, "{} already exists; not overwriting", path.display()),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for NewError {}
impl From<io::Error> for NewError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A package name Cargo accepts and that does not shadow Carmy or the standard library.
pub fn validate_name(name: &str) -> Result<(), NewError> {
    let invalid = |reason: &str| Err(NewError::InvalidName(format!("`{name}` {reason}")));
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
pub fn generate(parent: &Path, name: &str, carmy: &Dependency) -> Result<PathBuf, NewError> {
    validate_name(name)?;
    let root = parent.join(name);
    if root.exists() {
        return Err(NewError::Exists(root));
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
