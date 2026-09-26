use carmy_cli::{Dependency, NewError, generate, validate_name};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("carmy-cli-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn generates_conventional_layout() {
    let parent = scratch("layout");
    let root = generate(&parent, "shop", &Dependency::Path(workspace())).unwrap();
    for file in [
        "Cargo.toml",
        "carmy.toml",
        "README.md",
        ".gitignore",
        "src/main.rs",
        "src/tools/mod.rs",
        "src/tools/hello.rs",
    ] {
        assert!(root.join(file).is_file(), "missing {file}");
    }
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"shop\""));
    assert!(manifest.contains("crates/carmy\" }"), "{manifest}");
    assert!(
        !manifest.contains("{{"),
        "unrendered placeholder in {manifest}"
    );
    let root = generate(&parent, "shop-git", &Dependency::Git).unwrap();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("git = \"https://github.com/igorvieira/Carmy\""));
    let root = generate(&parent, "shop-crates", &Dependency::CratesIo).unwrap();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("carmy = \"0.1\""), "{manifest}");
}

#[test]
fn never_overwrites_and_validates_names() {
    let parent = scratch("guards");
    generate(&parent, "shop", &Dependency::Git).unwrap();
    assert!(matches!(
        generate(&parent, "shop", &Dependency::Git),
        Err(NewError::Exists(_))
    ));
    for bad in ["Shop", "1shop", "shop!", "carmy", "my shop", ""] {
        assert!(validate_name(bad).is_err(), "{bad} should be rejected");
    }
    for good in ["shop", "my_shop", "my-shop2"] {
        validate_name(good).unwrap();
    }
    assert!(generate(&parent, "x", &Dependency::Path(parent.clone())).is_err());
}

/// Slow: compiles the generated project. CI runs it with `--ignored`.
#[test]
#[ignore]
fn generated_project_passes_its_tests() {
    let parent = scratch("build");
    let root = generate(&parent, "shop", &Dependency::Path(workspace())).unwrap();
    let status = Command::new(env!("CARGO"))
        .arg("test")
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", workspace().join("target"))
        .status()
        .unwrap();
    assert!(status.success());
}
