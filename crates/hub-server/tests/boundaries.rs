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

/// The features a manifest defines, each with what it switches on.
fn features(manifest: &str) -> BTreeMap<String, Vec<String>> {
    let mut found = BTreeMap::new();
    let mut inside = false;
    let mut lines = manifest.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#'));

    while let Some(line) = lines.next() {
        if line.starts_with('[') {
            inside = line == "[features]";

            continue;
        }

        let Some((name, value)) = line.split_once('=').filter(|_| inside) else { continue };
        let mut list = value.to_string();

        // A list may run over several lines.
        while !list.contains(']') {
            let Some(more) = lines.next() else { break };
            list.push_str(more);
        }

        found.insert(name.trim().trim_matches('"').to_string(), list.split('"').skip(1).step_by(2).map(String::from).collect());
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

fn sources(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap_or_else(|error| panic!("{}: {error}", folder.display())) {
        let path = entry.expect("a folder entry").path();

        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// What a module's source says it depends on: the `depends_on: &[…]` of its `ModuleMeta`.
/// Read from the text, so that a module this build leaves out is checked like the others.
fn declared_in_source(folder: &str) -> Vec<String> {
    let mut files = Vec::new();
    sources(&root().join("modules").join(folder).join("src"), &mut files);

    let mut lists: Vec<Vec<String>> = Vec::new();

    for file in files {
        let text = read(&file);

        for (start, _) in text.match_indices("depends_on:") {
            let rest = &text[start..];
            let open = rest.find('[').unwrap_or_else(|| panic!("{}: depends_on is not written as a list", file.display()));
            let close = rest.find(']').unwrap_or_else(|| panic!("{}: depends_on is not written as a list", file.display()));

            assert!(open < close && rest[..open].trim_end().ends_with('&'), "{}: write depends_on as `depends_on: &[\"id\", …]`", file.display());
            lists.push(rest[open..close].split('"').skip(1).step_by(2).map(String::from).collect());
        }
    }

    assert_eq!(lists.len(), 1, "modules/{folder} states `depends_on` once, in its ModuleMeta");

    lists.remove(0)
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

        [features]
        default = ["wiki"]
        wiki = ["dep:hub-mod-wiki"]
        both = [
            "wiki",
            "dep:hub-mod-github",
        ]
    "#;

    assert_eq!(package_name(manifest), "hub-mod-sample");
    assert_eq!(dependencies(manifest, false), ["hub-core", "serde", "hub-mod-wiki", "hub-mod-github", "libc"]);
    assert_eq!(dependencies(manifest, true), ["hub-core", "serde", "hub-mod-wiki", "hub-mod-github", "libc", "tower"]);
    assert_eq!(features(manifest)["wiki"], ["dep:hub-mod-wiki"]);
    assert_eq!(features(manifest)["both"], ["wiki", "dep:hub-mod-github"]);
}

#[test]
fn a_module_depends_only_on_the_modules_it_declares() {
    let folders: Vec<String> = module_folders().into_iter().map(|(folder, _)| folder).collect();

    for (folder, manifest) in module_folders() {
        assert_eq!(package_name(&manifest), format!("hub-mod-{folder}"), "modules/{folder} holds the package hub-mod-{folder}");

        let needs = dependencies(&manifest, false);
        assert!(
            !needs.iter().any(|name| name == "hub-server"),
            "modules/{folder} depends on hub-server; the server depends on modules, never the reverse"
        );

        // Whether or not this build has the module: what it declares is in its source.
        let depends_on = declared_in_source(&folder);

        for other in needs.iter().filter_map(|name| name.strip_prefix("hub-mod-")) {
            assert!(
                depends_on.iter().any(|id| id == other),
                "modules/{folder} depends on the crate hub-mod-{other}, but its ModuleMeta::depends_on does not list \"{other}\""
            );
        }

        for id in &depends_on {
            assert!(folders.contains(id), "modules/{folder} declares that it depends on \"{id}\", and there is no modules/{id}");
        }
    }
}

#[test]
fn the_source_reader_agrees_with_the_modules_of_this_build() {
    for (id, depends_on) in declared() {
        assert_eq!(declared_in_source(&id), depends_on, "what modules/{id} declares in its source and what its ModuleMeta says");
    }
}

/// A module is one feature of the server, named by its id, that switches on its crate and
/// the features of the modules it depends on; `modules()` loads it under that feature.
#[test]
fn every_module_is_a_feature_of_the_server() {
    let manifest = read(&root().join("crates/hub-server/Cargo.toml"));
    let registry = read(&root().join("crates/hub-server/src/lib.rs"));
    let features = features(&manifest);
    let optional: Vec<&str> =
        manifest.lines().filter(|line| line.contains("optional = true")).filter_map(|line| line.split([' ', '=']).next()).collect();

    for (folder, _) in module_folders() {
        let package = format!("hub-mod-{folder}");
        let how = format!(
            "register modules/{folder} in crates/hub-server: `{folder} = [\"dep:{package}\"]` under [features], \
             `{package} = {{ path = \"../../modules/{folder}\", optional = true }}` under [dependencies], and its line in `modules()`"
        );
        let feature = features.get(&folder).unwrap_or_else(|| panic!("hub-server has no feature \"{folder}\"; {how}"));

        assert!(feature.contains(&format!("dep:{package}")), "the feature \"{folder}\" does not switch on {package}; {how}");
        assert!(optional.contains(&package.as_str()), "{package} is not an optional dependency of hub-server; {how}");

        for other in declared_in_source(&folder) {
            assert!(
                feature.contains(&other),
                "modules/{folder} depends on \"{other}\", so the feature \"{folder}\" must switch on the feature \"{other}\""
            );
        }

        let crate_name = package.replace('-', "_");
        let loaded = registry
            .split(&format!("#[cfg(feature = \"{folder}\")]"))
            .nth(1)
            .is_some_and(|after| after.trim_start().starts_with(&format!("modules.push(Arc::new({crate_name}::")));

        assert!(loaded, "hub_server::modules() does not load {crate_name} under #[cfg(feature = \"{folder}\")]; {how}");
    }

    for name in dependencies(&manifest, false).iter().filter(|name| name.starts_with("hub-mod-")) {
        assert!(optional.contains(&name.as_str()), "{name} must be an optional dependency of hub-server, switched on by its feature");
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
