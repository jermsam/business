//! Prevent dormant source and test files from silently escaping Cargo coverage.
use anyhow::{ensure, Result};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
fn rust_files(directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, output)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            output.push(path.canonicalize()?);
        }
    }
    Ok(())
}
fn visit(file: PathBuf, seen: &mut HashSet<PathBuf>) -> Result<()> {
    let file = file.canonicalize()?;
    if !seen.insert(file.clone()) {
        return Ok(());
    }
    let source = std::fs::read_to_string(&file)?;
    let modules = regex::Regex::new(
        r#"(?m)(?:#\[path\s*=\s*"([^"]+)"\]\s*)?(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"#,
    )?;
    for capture in modules.captures_iter(&source) {
        let next = if let Some(path) = capture.get(1) {
            file.parent().unwrap().join(path.as_str())
        } else {
            let parent = file.parent().unwrap();
            let directory = if matches!(
                file.file_stem().and_then(|s| s.to_str()),
                Some("lib" | "main" | "mod")
            ) {
                parent.to_path_buf()
            } else {
                parent.join(file.file_stem().unwrap())
            };
            let name = &capture[2];
            let plain = directory.join(format!("{name}.rs"));
            if plain.exists() {
                plain
            } else {
                directory.join(name).join("mod.rs")
            }
        };
        ensure!(
            next.exists(),
            "Declared module does not exist: {}",
            next.display()
        );
        visit(next, seen)?;
    }
    Ok(())
}
#[test]
fn every_rust_source_and_test_is_connected_to_a_cargo_target() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut seen = HashSet::new();
    visit(root.join("src/lib.rs"), &mut seen)?;
    visit(root.join("src/main.rs"), &mut seen)?;
    for directory in ["src/bin", "tests"] {
        for entry in std::fs::read_dir(root.join(directory))? {
            let path = entry?.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
                visit(path, &mut seen)?;
            } else if directory == "tests" && path.join("main.rs").exists() {
                visit(path.join("main.rs"), &mut seen)?;
            }
        }
    }
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files)?;
    rust_files(&root.join("tests"), &mut files)?;
    let missing: Vec<_> = files
        .iter()
        .filter(|f| !seen.contains(*f))
        .map(|f| f.strip_prefix(root).unwrap().display().to_string())
        .collect();
    ensure!(
        missing.is_empty(),
        "Rust files outside the active module graph: {missing:?}"
    );
    Ok(())
}
#[test]
fn every_service_uses_the_common_dogrs_modules() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services");
    fn walk(directory: &Path) -> Result<()> {
        let entries: Vec<_> =
            std::fs::read_dir(directory)?.collect::<std::result::Result<_, _>>()?;
        let prefix = directory
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace('-', "_");
        if directory.join(format!("{prefix}_service.rs")).exists() {
            for suffix in ["service", "shared", "hooks", "schema"] {
                ensure!(
                    directory.join(format!("{prefix}_{suffix}.rs")).is_file(),
                    "Missing {prefix}_{suffix}.rs"
                );
            }
            ensure!(directory.join("mod.rs").is_file(), "Missing service mod.rs");
        }
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path)?;
            }
        }
        Ok(())
    }
    walk(&root)
}
