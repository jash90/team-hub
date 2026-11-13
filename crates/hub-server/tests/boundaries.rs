//! The dependency rules of the workspace, read from the manifests, so that crossing a
//! boundary fails the build instead of contradicting a paragraph of the documentation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn quoted(text: &str) -> Option<&str> {
    let start = text.find('"')? + 1;
    let end = start + text[start..].find('"')?;

    Some(&text[start..end])
}

/// The package a manifest declares.
fn package_name(manifest: &str) -> String {
    let mut section = "";

    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if section == "[package]" && line.starts_with("name") {
            return quoted(line).unwrap_or_default().to_string();
        }
    }

    String::new()
}

/// The packages a manifest depends on. With `dev`, those its tests depend on as well.
fn dependencies(manifest: &str, dev: bool) -> Vec<String> {
    let wanted = |table: &str| table == "dependencies" || table == "build-dependencies" || (dev && table == "dev-dependencies");
    let mut found = Vec::new();
    let mut inside = false;
    // `[dependencies.name]` spells one dependency as a table of its own.
    let mut own_table = false;

    for line in manifest.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        if line.starts_with('[') {
            let header = line.trim_matches(|character| character == '[' || character == ']');
            let parts: Vec<&str> = header.split('.').collect();
            let table = parts.iter().position(|part| wanted(part));

            inside = table == Some(parts.len() - 1);
            own_table = table.is_some_and(|index| index + 2 == parts.len());

            if own_table {
                found.push(parts[parts.len() - 1].trim_matches('"').to_string());
            }

            continue;
        }

        let Some((key, value)) = line.split_once('=') else { continue };

        if own_table && key.trim() == "package" {
            found.pop();
            found.push(quoted(value).unwrap_or_default().to_string());
        } else if inside {
            // `name = "1"`, `name.workspace = true`, `alias = { package = "name", … }`
            let renamed = value.split("package").nth(1).and_then(quoted);
            let name = key.trim().trim_matches('"').split('.').next().unwrap_or_default();

            found.push(renamed.unwrap_or(name).to_string());
        }
    }

    found
}

/// What each module compiled into this build says it depends on, by id.
fn declared() -> BTreeMap<String, Vec<String>> {
    hub_server::modules()
        .iter()
        .map(|module| (module.meta().id.to_string(), module.meta().depends_on.iter().map(|id| id.to_string()).collect()))
        .collect()
}

fn module_folders() -> Vec<(String, String)> {
    let mut folders = Vec::new();

    for entry in std::fs::read_dir(root().join("modules")).expect("the modules folder") {
        let path = entry.expect("a folder entry").path();
        let manifest = path.join("Cargo.toml");

        if manifest.is_file() {
            folders.push((path.file_name().unwrap_or_default().to_string_lossy().into_owned(), read(&manifest)));
        }
    }

    folders.sort();

    folders
}

#[test]
fn the_manifest_reader_sees_every_way_of_writing_a_dependency() {
    let manifest = r#"
        [package]
        name = "hub-mod-sample"

        [dependencies]
        hub-core.workspace = true
        serde = { version = "1", features = ["derive"] }
        pages = { package = "hub-mod-wiki", path = "../wiki" }

        [dependencies.hub-mod-github]
        path = "../github"

        [target.'cfg(unix)'.dependencies]
        libc = "0.2"

        [dev-dependencies]
        tower = "0.5"
    "#;

    assert_eq!(package_name(manifest), "hub-mod-sample");
    assert_eq!(dependencies(manifest, false), ["hub-core", "serde", "hub-mod-wiki", "hub-mod-github", "libc"]);
    assert_eq!(dependencies(manifest, true), ["hub-core", "serde", "hub-mod-wiki", "hub-mod-github", "libc", "tower"]);
}

#[test]
fn a_module_depends_only_on_the_modules_it_declares() {
    let declared = declared();

    for (folder, manifest) in module_folders() {
        assert_eq!(package_name(&manifest), format!("hub-mod-{folder}"), "modules/{folder} holds the package hub-mod-{folder}");

        let needs = dependencies(&manifest, false);
        assert!(
            !needs.iter().any(|name| name == "hub-server"),
            "modules/{folder} depends on hub-server; the server depends on modules, never the reverse"
        );

        let others: Vec<&str> = needs.iter().filter_map(|name| name.strip_prefix("hub-mod-")).collect();

        // A module that this build leaves out cannot be asked what it declares.
        let Some(depends_on) = declared.get(&folder) else {
            assert!(
                others.is_empty(),
                "modules/{folder} depends on {others:?} but is not in hub_server::modules(), so what it declares cannot be checked"
            );

            continue;
        };

        for other in others {
            assert!(
                depends_on.iter().any(|id| id == other),
                "modules/{folder} depends on the crate hub-mod-{other}, but its ModuleMeta::depends_on does not list \"{other}\""
            );
        }
    }
}

#[test]
fn every_module_of_the_build_lives_in_its_own_folder() {
    let folders: Vec<String> = module_folders().into_iter().map(|(folder, _)| folder).collect();

    for id in declared().keys() {
        assert!(folders.contains(id), "the module \"{id}\" has no folder modules/{id}");
    }
}

#[test]
fn hub_core_is_rules_and_types_only() {
    const IMPURE: &[&str] = &["rusqlite", "axum", "tokio", "ureq", "hyper", "reqwest", "tower", "tower-http", "tracing-subscriber"];

    for name in dependencies(&read(&root().join("crates/hub-core/Cargo.toml")), true) {
        assert!(!IMPURE.contains(&name.as_str()), "hub-core depends on {name}: it has no network, no disk and no runtime");
        assert!(!name.starts_with("hub-"), "hub-core depends on {name}: everything depends on hub-core, never the reverse");
    }
}

#[test]
fn hub_app_knows_no_module() {
    for name in dependencies(&read(&root().join("crates/hub-app/Cargo.toml")), true) {
        assert!(!name.starts_with("hub-mod-"), "hub-app depends on {name}: modules plug into the host, the host does not know them");
        assert_ne!(name, "hub-server", "hub-app depends on hub-server");
    }
}
