use carmy_cli::{
    Dependency, Error, find_project, generate, generate_tool, validate_name, validate_tool_name,
};
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
    let minor = env!("CARGO_PKG_VERSION").rsplitn(2, '.').last().unwrap();
    assert!(
        manifest.contains(&format!("carmy = \"{minor}\"")),
        "{manifest}"
    );
}

#[test]
fn never_overwrites_and_validates_names() {
    let parent = scratch("guards");
    generate(&parent, "shop", &Dependency::Git).unwrap();
    assert!(matches!(
        generate(&parent, "shop", &Dependency::Git),
        Err(Error::Exists(_))
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
    for (name, effect) in [
        ("search_products", "read"),
        ("create_order", "write"),
        ("delete_customer", "destructive"),
    ] {
        generate_tool(&root, name, effect, "A \"generated\" tool").unwrap();
    }
    // Generated code must already be formatted, so `cargo fmt --check` passes from day one.
    let fmt = Command::new(env!("CARGO"))
        .args(["fmt", "--check"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(fmt.success(), "generated code is not rustfmt-clean");
    // Agents speak carmy-console/1 to the generated application over stdio.
    let mut console = Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--", "console"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", workspace().join("target"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let mut stdin = console.stdin.take().unwrap();
        writeln!(stdin, r#"{{"id":1,"op":"tools"}}"#).unwrap();
        writeln!(
            stdin,
            r#"{{"id":2,"op":"call","tool":"hello","arguments":{{"name":"Ada"}}}}"#
        )
        .unwrap();
    }
    let output = console.wait_with_output().unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["protocol"], "carmy-console/1");
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 4);
    assert_eq!(lines[2]["result"]["data"]["message"], "Hello, Ada!");
    let status = Command::new(env!("CARGO"))
        .arg("test")
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", workspace().join("target"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn generates_tools_into_the_project() {
    let parent = scratch("tool");
    let root = generate(&parent, "shop", &Dependency::Git).unwrap();
    let file = generate_tool(&root, "create_order", "write", "Place an order").unwrap();
    assert_eq!(file, root.join("src/tools/create_order.rs"));
    let code = fs::read_to_string(&file).unwrap();
    assert!(code.contains("struct CreateOrderInput"));
    assert!(code.contains("effect = \"write\""));
    assert!(code.contains("description = \"Place an order\""));
    assert!(code.contains("async fn create_order(input: CreateOrderInput)"));
    let modules = fs::read_to_string(root.join("src/tools/mod.rs")).unwrap();
    assert!(
        modules.contains("mod create_order;\nmod hello;\n"),
        "{modules}"
    );

    let destructive = generate_tool(&root, "delete_customer", "destructive", "Delete").unwrap();
    let code = fs::read_to_string(destructive).unwrap();
    assert!(code.contains("confirmation = \"required\""));
    assert!(code.contains("CONFIRMATION_REQUIRED"));
    let read = generate_tool(&root, "search", "read", "Search").unwrap();
    let code = fs::read_to_string(read).unwrap();
    assert!(code.contains("idempotent = true") && code.contains("parallel_safe = true"));
}

#[test]
fn tool_generation_is_guarded() {
    let parent = scratch("tool-guards");
    let root = generate(&parent, "shop", &Dependency::Git).unwrap();
    assert!(matches!(
        generate_tool(&root, "hello", "read", "again"),
        Err(Error::Exists(_))
    ));
    assert!(matches!(
        generate_tool(&root, "search", "sideways", "x"),
        Err(Error::InvalidEffect(_))
    ));
    for bad in ["Search", "search-products", "1search", "fn", "tests", ""] {
        assert!(validate_tool_name(bad).is_err(), "{bad} should be rejected");
    }
    assert!(matches!(find_project(&parent), Err(Error::NotAProject(_))));
    assert_eq!(find_project(&root.join("src/tools")).unwrap(), root);
}

#[test]
fn cli_generates_from_a_subdirectory() {
    let parent = scratch("tool-cli");
    let root = generate(&parent, "shop", &Dependency::Git).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_carmy"))
        .args(["g", "tool", "ping", "--effect", "none"])
        .current_dir(root.join("src"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("src/tools/ping.rs").is_file());
    let missing_effect = Command::new(env!("CARGO_BIN_EXE_carmy"))
        .args(["g", "tool", "pong"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!missing_effect.status.success());
    assert!(String::from_utf8_lossy(&missing_effect.stderr).contains("--effect"));
}
