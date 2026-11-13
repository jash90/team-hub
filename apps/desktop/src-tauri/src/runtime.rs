//! The agent runtime: the Claude Agent SDK and its `claude` program. An installer does not
//! carry them. The app fetches them once, on the person's computer and with the person's own
//! npm, into its data folder, and runs the helper from there:
//!
//! ```text
//! <data>/agent-runtime/
//!     <hash of package.json + package-lock.json>/     one per set of dependencies
//!         package.json  package-lock.json             copies of the bundled ones
//!         node_modules/                               what `npm ci` put there
//!         .complete                                   written last, before the folder gets its name
//!         helper-<hash of the helper's source>/       main.ts, hub-tools.ts: what Node runs
//!     <hash>.partial-<pid>/                           an install under way, or one that was interrupted
//!     state.json                                      "downloading" or "failed", for the launcher's message
//!     install.log                                     what npm printed the last time
//! ```
//!
//! npm works in a `.partial` folder that is renamed when everything is there, so a folder
//! with the real name is never half an install. An update of the app with the same lock file
//! finds the folder and downloads nothing; with another lock file it fetches into a new
//! folder and removes the old one. The helper's source sits beside `node_modules` because
//! Node resolves a module's imports from the folders above the module, and has no other way
//! for `import` (NODE_PATH is for `require` only).
//!
//! Nothing here needs Tauri, so `rustc --test src/runtime.rs` runs its tests alone.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub const FOLDER: &str = "agent-runtime";
const COMPLETE: &str = ".complete";
const STATE: &str = "state.json";
const LOG: &str = "install.log";
const SDK: &str = "node_modules/@anthropic-ai/claude-agent-sdk/package.json";

/// What npm is asked to do. Nothing names a registry: it is the one npm is configured with.
/// No package of the helper has an install script, and none is run; no links in `.bin`,
/// which nothing uses; `http` makes npm print a line per download, which is the progress shown.
pub const NPM_ARGUMENTS: [&str; 7] = ["ci", "--omit=dev", "--no-audit", "--no-fund", "--ignore-scripts", "--no-bin-links", "--loglevel=http"];

/// What to tell the person the download is.
pub const WHAT: &str = "the agent runtime (the Claude Agent SDK and its program, about 350 MB on disk, once)";

/// FNV-1a, 64 bits, written out here so that the folder's name never changes with the
/// compiler: a name that changed would download everything again.
fn hash(parts: &[(&str, &[u8])]) -> String {
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };

    for (name, bytes) in parts {
        feed(name.as_bytes());
        feed(&[0]);
        feed(bytes);
        feed(&[0]);
    }

    format!("{state:016x}")
}

fn json_text(text: &str) -> String {
    let mut out = String::from("\"");

    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }

    out.push('"');
    out
}

/// A folder made in full under another name first, so that it is there whole or not at all.
fn place(folder: &Path, fill: impl FnOnce(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    let name = folder.file_name().and_then(|name| name.to_str()).ok_or(std::io::ErrorKind::InvalidInput)?;
    let partial = folder.with_file_name(format!("{name}.partial-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)?;

    let outcome = fill(&partial).and_then(|()| rename(&partial, folder));

    if outcome.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }

    outcome
}

/// Windows refuses to rename a folder while a virus scanner still looks at what was just
/// written into it, for a moment.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;

    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if tries == 10 => return Err(error),
            Err(_) => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
}

/// The npm that belongs to the Node of the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Npm {
    pub program: PathBuf,
    /// Node's own folder, when the settings name Node by its path: npm is a script that
    /// starts `node`, and has to find that one.
    pub beside_node: Option<PathBuf>,
}

impl Npm {
    pub fn for_node(node: &str) -> Self {
        let name = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let folder = Path::new(node).parent().filter(|folder| !folder.as_os_str().is_empty());

        match folder {
            Some(folder) if folder.join(name).is_file() => Self { program: folder.join(name), beside_node: Some(folder.to_path_buf()) },
            _ => Self { program: PathBuf::from(name), beside_node: None },
        }
    }

    fn command(&self, folder: &Path) -> Command {
        let mut command = Command::new(&self.program);

        command.args(NPM_ARGUMENTS).current_dir(folder).env("npm_config_update_notifier", "false");
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(beside) = &self.beside_node {
            let rest = std::env::var_os("PATH").unwrap_or_default();
            let folders = std::iter::once(beside.clone()).chain(std::env::split_paths(&rest));

            if let Ok(path) = std::env::join_paths(folders) {
                command.env("PATH", path);
            }
        }

        // A window has no console; without this Windows opens one while npm runs.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            command.creation_flags(0x0800_0000);
        }

        command
    }
}

/// Why a download did not end with a runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// One sentence for a person.
    pub reason: String,
    /// The last lines npm printed, when it ran.
    pub detail: String,
}

/// What npm's output says went wrong, in words for a person.
fn reason_for(output: &str, code: Option<i32>) -> String {
    let has = |words: &[&str]| words.iter().any(|word| output.contains(word));

    if has(&["ENOTFOUND", "EAI_AGAIN", "ETIMEDOUT", "ECONNREFUSED", "ECONNRESET", "ENETUNREACH", "network request"]) {
        "the npm registry could not be reached. Is this computer online, and does npm need a proxy?".to_string()
    } else if has(&["E401", "E403", "E407", "ENEEDAUTH"]) {
        "the npm registry refused the request. Check npm's configuration: the registry, a proxy, a login.".to_string()
    } else if has(&["ENOSPC"]) {
        "the disk is full.".to_string()
    } else if has(&["EACCES", "EPERM"]) {
        "npm was not allowed to write. Check who owns the app's data folder and npm's cache.".to_string()
    } else if has(&["EINTEGRITY"]) {
        "what the npm registry sent is not what the app's lock file expects.".to_string()
    } else {
        match code {
            Some(code) => format!("npm ended with exit code {code}."),
            None => "npm was stopped before it finished.".to_string(),
        }
    }
}

/// Where the runtime of this app is, or will be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The helper as the installer carries it: `package.json`, `package-lock.json`, `src/`, `launch.mjs`.
    bundled: PathBuf,
    /// `<data>/agent-runtime`.
    root: PathBuf,
    /// `<root>/<hash of the dependencies>`.
    runtime: PathBuf,
    /// `<runtime>/helper-<hash of the source>`.
    helper: PathBuf,
}

impl Plan {
    /// Reads what the installer carries and names the folders after it. `data` is the app's
    /// data folder, the one that also holds `state` and `worktrees`.
    pub fn new(bundled: &Path, data: &Path) -> std::io::Result<Self> {
        let manifest = std::fs::read(bundled.join("package.json"))?;
        let lock = std::fs::read(bundled.join("package-lock.json"))?;
        let mut sources = Self::sources(bundled)?
            .into_iter()
            .map(|file| Ok((file.clone(), std::fs::read(bundled.join("src").join(&file))?)))
            .collect::<std::io::Result<Vec<_>>>()?;

        sources.sort();

        if !sources.iter().any(|(name, _)| name == "main.ts") {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{} has no src/main.ts", bundled.display())));
        }

        let root = data.join(FOLDER);
        let runtime = root.join(hash(&[("package.json", &manifest), ("package-lock.json", &lock)]));
        let parts: Vec<(&str, &[u8])> = sources.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
        let helper = runtime.join(format!("helper-{}", hash(&parts)));

        Ok(Self { bundled: bundled.to_path_buf(), root, runtime, helper })
    }

    /// The names of the helper's source files.
    fn sources(bundled: &Path) -> std::io::Result<Vec<String>> {
        let mut names = Vec::new();

        for entry in std::fs::read_dir(bundled.join("src"))? {
            let name = entry?.file_name().to_string_lossy().into_owned();

            if name.ends_with(".ts") {
                names.push(name);
            }
        }

        Ok(names)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn folder(&self) -> &Path {
        &self.runtime
    }

    pub fn log(&self) -> PathBuf {
        self.root.join(LOG)
    }

    /// The script in the installer that starts the helper, or says why it cannot.
    pub fn launcher(&self) -> PathBuf {
        self.bundled.join("launch.mjs")
    }

    /// The helper's script beside the fetched `node_modules`: what the launcher is told to run.
    pub fn script(&self) -> PathBuf {
        self.helper.join("main.ts")
    }

    /// Whether the dependencies are there, whole. Looks at the disk only.
    pub fn is_complete(&self) -> bool {
        self.runtime.join(COMPLETE).is_file() && self.runtime.join(SDK).is_file()
    }

    /// Whether a thread can start: the dependencies and this app's helper beside them.
    pub fn is_ready(&self) -> bool {
        self.is_complete() && self.script().is_file()
    }

    /// Puts this app's helper source beside the dependencies, and removes the source of
    /// other versions of the app. Needs no network.
    pub fn place_helper(&self) -> std::io::Result<()> {
        if !self.script().is_file() {
            let _ = std::fs::remove_dir_all(&self.helper);

            place(&self.helper, |partial| {
                for name in Self::sources(&self.bundled)? {
                    std::fs::copy(self.bundled.join("src").join(&name), partial.join(&name))?;
                }

                Ok(())
            })?;
        }

        for entry in std::fs::read_dir(&self.runtime)?.flatten() {
            if entry.file_name().to_string_lossy().starts_with("helper-") && entry.path() != self.helper {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }

        Ok(())
    }

    /// The command npm is run with, as a person would type it.
    pub fn command_line(&self, npm: &Npm) -> String {
        format!("{} {}", npm.program.display(), NPM_ARGUMENTS.join(" "))
    }

    fn write_state(&self, state: &str, reason: &str) {
        let text = format!("{{\"state\":{},\"pid\":{},\"reason\":{}}}\n", json_text(state), std::process::id(), json_text(reason));

        let _ = std::fs::write(self.root.join(STATE), text);
    }

    /// Everything in the root that is not the runtime of this app: installs that were
    /// interrupted, and the runtimes of other versions.
    fn remove_others(&self, also_whole_ones: bool) {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let partial = name.contains(".partial-");

            if entry.path() != self.runtime && entry.path().is_dir() && (partial || also_whole_ones) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    fn fail(&self, reason: String, detail: String) -> Failure {
        self.write_state("failed", &reason);

        Failure { reason, detail }
    }

    /// Fetches the dependencies with npm and gives the folder its name when all of it is
    /// there. `said` gets every line npm prints. On failure nothing with the runtime's name
    /// is left, and the reason is kept for the launcher to repeat.
    pub fn install(&self, npm: &Npm, said: &mut dyn FnMut(&str)) -> Result<(), Failure> {
        let prepared = std::fs::create_dir_all(&self.root).and_then(|()| {
            self.remove_others(false);

            // A folder with the name but not whole (the marker or the SDK gone) starts over.
            if self.runtime.exists() {
                std::fs::remove_dir_all(&self.runtime)?;
            }

            Ok(())
        });

        if let Err(error) = prepared {
            return Err(self.fail(format!("the folder {} could not be prepared ({error}).", self.root.display()), String::new()));
        }

        self.write_state("downloading", "");

        let mut failure = None;
        let placed = place(&self.runtime, |partial| match self.fetch_into(partial, npm, said) {
            Ok(()) => std::fs::write(partial.join(COMPLETE), "npm ci finished and the Claude Agent SDK is here.\n"),
            Err(found) => {
                failure = Some(found);

                Err(std::io::ErrorKind::Other.into())
            }
        });

        match (placed, failure) {
            (Ok(()), _) => {}
            (Err(_), Some((reason, detail))) => return Err(self.fail(reason, detail)),
            (Err(error), None) => return Err(self.fail(format!("the downloaded files could not be put in place ({error})."), String::new())),
        }

        self.remove_others(true);

        if let Err(error) = self.place_helper() {
            return Err(self.fail(format!("the helper could not be copied beside the runtime ({error})."), String::new()));
        }

        let _ = std::fs::remove_file(self.root.join(STATE));

        Ok(())
    }

    /// `npm ci` in `partial`, with copies of the bundled manifest and lock file.
    fn fetch_into(&self, partial: &Path, npm: &Npm, said: &mut dyn FnMut(&str)) -> Result<(), (String, String)> {
        let copied =
            ["package.json", "package-lock.json"].iter().try_for_each(|name| std::fs::copy(self.bundled.join(name), partial.join(name)).map(|_| ()));

        if let Err(error) = copied {
            return Err((format!("the app's own files could not be copied ({error})."), String::new()));
        }

        let mut log = std::fs::File::create(self.log()).ok();
        let mut note = |line: &str| {
            if let Some(log) = log.as_mut() {
                let _ = writeln!(log, "{line}");
            }
        };

        note(&format!("$ cd {} && {}", partial.display(), self.command_line(npm)));

        let mut child = match npm.command(partial).spawn() {
            Ok(child) => child,
            Err(error) => {
                note(&format!("npm could not be started: {error}"));

                return Err((
                    format!(
                        "npm could not be started (\"{}\": {error}). npm comes with Node.js: install Node.js 22.18 or newer from https://nodejs.org.",
                        npm.program.display()
                    ),
                    String::new(),
                ));
            }
        };
        let (line_out, lines) = mpsc::channel::<String>();
        let read = |stream: Option<Box<dyn Read + Send>>, to: mpsc::Sender<String>| {
            std::thread::spawn(move || {
                if let Some(stream) = stream {
                    for line in BufReader::new(stream).lines().map_while(Result::ok) {
                        let _ = to.send(line);
                    }
                }
            })
        };

        read(child.stdout.take().map(|out| Box::new(out) as Box<dyn Read + Send>), line_out.clone());
        read(child.stderr.take().map(|err| Box::new(err) as Box<dyn Read + Send>), line_out);

        let mut last = VecDeque::new();
        let mut all = String::new();

        for line in lines {
            note(&line);
            said(&line);
            all.push_str(&line);
            all.push('\n');

            if last.len() == 8 {
                last.pop_front();
            }

            last.push_back(line);
        }

        let code = child.wait().ok().and_then(|status| if status.success() { Some(0) } else { status.code() });
        let detail = last.into_iter().collect::<Vec<_>>().join("\n");

        if code != Some(0) {
            return Err((reason_for(&all, code), detail));
        }

        keep_one_libc(partial);

        if !partial.join(SDK).is_file() || !has_program(partial) {
            return Err((
                "npm finished, but the Claude Agent SDK or its program for this computer is not among what it installed.".to_string(),
                detail,
            ));
        }

        Ok(())
    }
}

/// On Linux npm installs the program for glibc and the one for musl: the lock file does not
/// say which is which. This app is built for one of them, and so is the Node it works with.
fn keep_one_libc(folder: &Path) {
    let unused: fn(&str) -> bool = if cfg!(all(target_os = "linux", target_env = "musl")) {
        |name: &str| !name.ends_with("-musl")
    } else if cfg!(target_os = "linux") {
        |name: &str| name.ends_with("-musl")
    } else {
        return;
    };

    for (name, path) in programs(folder) {
        let twin = if name.ends_with("-musl") { name.trim_end_matches("-musl").to_string() } else { format!("{name}-musl") };

        if unused(&name) && path.with_file_name(twin).is_dir() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// The packages that hold a native program: `@anthropic-ai/claude-agent-sdk-<system>`.
fn programs(folder: &Path) -> Vec<(String, PathBuf)> {
    let scope = folder.join("node_modules").join("@anthropic-ai");
    let Ok(entries) = std::fs::read_dir(scope) else { return Vec::new() };

    entries
        .flatten()
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .filter(|(name, _)| name.starts_with("claude-agent-sdk-"))
        .collect()
}

fn has_program(folder: &Path) -> bool {
    programs(folder).iter().any(|(_, path)| ["claude", "claude.exe"].iter().any(|name| path.join(name).metadata().is_ok_and(|found| found.len() > 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the system's temporary one, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let folder = std::env::temp_dir().join(format!("team-hub-runtime-{name}-{}", std::process::id()));

            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).unwrap();

            Self(folder)
        }

        fn write(&self, file: &str, text: &str) -> PathBuf {
            let path = self.0.join(file);

            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();

            path
        }

        /// What an installer carries, with a lock file that says `lock` and a helper that says `source`.
        fn bundle(&self, lock: &str, source: &str) -> PathBuf {
            self.write("bundle/package.json", "{\"type\":\"module\"}");
            self.write("bundle/package-lock.json", lock);
            self.write("bundle/src/main.ts", source);
            self.write("bundle/src/hub-tools.ts", "export {};");
            self.write("bundle/src/notes.txt", "not source");
            self.write("bundle/launch.mjs", "");

            self.0.join("bundle")
        }

        fn plan(&self, lock: &str, source: &str) -> Plan {
            Plan::new(&self.bundle(lock, source), &self.0.join("data")).unwrap()
        }

        /// An npm that does what the script says, in the folder it is started in.
        #[cfg(unix)]
        fn npm(&self, script: &str) -> Npm {
            use std::os::unix::fs::PermissionsExt;

            let path = self.write("bin/npm", &format!("#!/bin/sh\n{script}\n"));

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

            Npm { program: path, beside_node: None }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What a working npm leaves behind, as far as this module looks.
    #[cfg(unix)]
    const INSTALLS: &str = "echo \"npm http fetch GET 200 sdk\" >&2
mkdir -p node_modules/@anthropic-ai/claude-agent-sdk node_modules/@anthropic-ai/claude-agent-sdk-linux-x64 node_modules/@anthropic-ai/claude-agent-sdk-linux-x64-musl
echo '{}' > node_modules/@anthropic-ai/claude-agent-sdk/package.json
echo program > node_modules/@anthropic-ai/claude-agent-sdk-linux-x64/claude
echo program > node_modules/@anthropic-ai/claude-agent-sdk-linux-x64-musl/claude
echo \"added 2 packages in 1s\"";

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(folder).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();

        names.sort();
        names
    }

    #[test]
    fn the_hash_is_the_published_fnv_and_tells_parts_apart() {
        // FNV-1a 64 of nothing but the separators would not be a known value; of plain bytes it is.
        let plain = |bytes: &[u8]| {
            let mut state: u64 = 0xcbf2_9ce4_8422_2325;

            for byte in bytes {
                state = (state ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }

            state
        };

        assert_eq!(plain(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(plain(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(plain(b"foobar"), 0x85944171f73967e8);
        assert_eq!(hash(&[("a", b"")]), format!("{:016x}", plain(b"a\0\0")));

        assert_eq!(hash(&[("lock", b"1")]).len(), 16);
        assert_ne!(hash(&[("lock", b"1")]), hash(&[("lock", b"2")]));
        // Where one part ends and the next begins is part of what is hashed.
        assert_ne!(hash(&[("ab", b"c")]), hash(&[("a", b"bc")]));
    }

    #[test]
    fn the_folders_are_named_after_the_lock_file_and_the_source() {
        let scratch = Scratch::new("names");
        let first = scratch.plan("lock 1", "source 1");
        let same = scratch.plan("lock 1", "source 1");
        let other_source = scratch.plan("lock 1", "source 2");
        let other_lock = scratch.plan("lock 2", "source 1");

        assert_eq!(first, same);
        assert_eq!(first.root(), scratch.0.join("data").join("agent-runtime"));
        assert_eq!(first.folder().parent(), Some(first.root()));
        assert!(first.script().starts_with(first.folder()));
        assert!(first.script().ends_with("main.ts"));
        assert_eq!(first.launcher(), scratch.0.join("bundle").join("launch.mjs"));

        // The same dependencies: the same download. Another helper: only another copy of it.
        assert_eq!(first.folder(), other_source.folder());
        assert_ne!(first.script(), other_source.script());
        // Other dependencies: another download.
        assert_ne!(first.folder(), other_lock.folder());
    }

    #[test]
    fn a_bundle_without_the_helper_is_not_planned() {
        let scratch = Scratch::new("no-helper");
        let bundle = scratch.bundle("lock", "source");

        std::fs::remove_file(bundle.join("src/main.ts")).unwrap();

        assert!(Plan::new(&bundle, &scratch.0.join("data")).is_err());
        assert!(Plan::new(&scratch.0.join("nowhere"), &scratch.0.join("data")).is_err());
    }

    #[test]
    fn only_a_marked_folder_with_the_sdk_counts_as_whole() {
        let scratch = Scratch::new("whole");
        let plan = scratch.plan("lock", "source");
        let relative = |file: &str| format!("data/agent-runtime/{}/{file}", plan.folder().file_name().unwrap().to_string_lossy());

        assert!(!plan.is_complete());

        // What an interrupted copy, or a person tidying up, could leave.
        scratch.write(&relative(SDK), "{}");
        assert!(!plan.is_complete(), "the SDK without the marker is not a finished install");

        std::fs::remove_dir_all(plan.folder()).unwrap();
        scratch.write(&relative(COMPLETE), "");
        assert!(!plan.is_complete(), "the marker without the SDK is not a runtime");

        scratch.write(&relative(SDK), "{}");
        assert!(plan.is_complete());
        assert!(!plan.is_ready(), "whole, but this app's helper is not beside it yet");

        plan.place_helper().unwrap();
        assert!(plan.is_ready());
    }

    #[test]
    fn the_helper_is_copied_beside_the_dependencies_and_old_copies_go() {
        let scratch = Scratch::new("helper");
        let old = scratch.plan("lock", "source of the last version");

        std::fs::create_dir_all(old.folder()).unwrap();
        old.place_helper().unwrap();
        assert_eq!(std::fs::read_to_string(old.script()).unwrap(), "source of the last version");
        assert_eq!(names(old.script().parent().unwrap()), ["hub-tools.ts", "main.ts"], "only the TypeScript is copied");

        let new = scratch.plan("lock", "source of this version");

        new.place_helper().unwrap();
        assert_eq!(std::fs::read_to_string(new.script()).unwrap(), "source of this version");
        assert!(!old.script().exists(), "the helper of the version before is removed");
        assert!(names(new.folder()).iter().all(|name| !name.contains(".partial-")), "{:?}", names(new.folder()));

        // Asked again, nothing changes.
        new.place_helper().unwrap();
        assert_eq!(names(new.folder()).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn an_install_ends_with_one_whole_folder_and_nothing_else() {
        let scratch = Scratch::new("install");
        let old = scratch.plan("the lock of the last version", "source");

        old.install(&scratch.npm(INSTALLS), &mut |_| {}).unwrap();
        assert!(old.is_ready());

        // What an install that was cut off leaves behind.
        scratch.write("data/agent-runtime/0123456789abcdef.partial-1/node_modules/half", "");

        let plan = scratch.plan("the lock of this version", "source");
        let mut heard = Vec::new();

        assert!(!plan.is_complete(), "another lock file: the old runtime is not this one's");
        plan.install(&scratch.npm(INSTALLS), &mut |line| heard.push(line.to_string())).unwrap();

        assert!(plan.is_ready());
        assert!(heard.iter().any(|line| line.contains("npm http fetch")), "{heard:?}");
        assert!(heard.iter().any(|line| line.contains("added 2 packages")), "{heard:?}");
        assert_eq!(
            names(plan.root()),
            [plan.folder().file_name().unwrap().to_string_lossy().into_owned(), LOG.to_string()],
            "the old runtime, the cut-off install and the state are gone"
        );
        assert!(std::fs::read_to_string(plan.log()).unwrap().contains("added 2 packages"));
        assert_eq!(std::fs::read_to_string(plan.folder().join("package-lock.json")).unwrap(), "the lock of this version");

        // One libc only, where there are two.
        let kept: Vec<String> = programs(plan.folder()).into_iter().map(|(name, _)| name).collect();

        if cfg!(target_os = "linux") {
            assert_eq!(kept.len(), 1, "{kept:?}");
            assert_eq!(kept[0].ends_with("-musl"), cfg!(target_env = "musl"), "{kept:?}");
        } else {
            assert_eq!(kept.len(), 2, "{kept:?}");
        }

        // With the runtime there, nothing is asked of npm again: whole is decided on disk.
        assert!(plan.is_ready());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_install_leaves_no_folder_that_looks_whole() {
        let scratch = Scratch::new("failed");
        let plan = scratch.plan("lock", "source");
        let failing = scratch.npm("mkdir -p node_modules/@anthropic-ai/claude-agent-sdk\necho '{}' > node_modules/@anthropic-ai/claude-agent-sdk/package.json\necho 'npm error code ENOTFOUND' >&2\necho 'npm error network request to https://registry.example/zod failed' >&2\nexit 1");
        let failure = plan.install(&failing, &mut |_| {}).unwrap_err();

        assert!(failure.reason.contains("could not be reached"), "{failure:?}");
        assert!(failure.detail.contains("ENOTFOUND"), "{failure:?}");
        assert!(!plan.is_complete());
        assert!(!plan.folder().exists(), "half an install must not have the runtime's name");
        assert_eq!(names(plan.root()), [LOG, STATE], "nothing of the attempt is left but its log and its reason");

        let state = std::fs::read_to_string(plan.root().join(STATE)).unwrap();

        assert!(state.contains("\"state\":\"failed\""), "{state}");
        assert!(state.contains("could not be reached"), "{state}");

        // npm said it worked, but the program is not there: still not a runtime.
        let hollow =
            scratch.npm("mkdir -p node_modules/@anthropic-ai/claude-agent-sdk\necho '{}' > node_modules/@anthropic-ai/claude-agent-sdk/package.json");
        let failure = plan.install(&hollow, &mut |_| {}).unwrap_err();

        assert!(failure.reason.contains("is not among what it installed"), "{failure:?}");
        assert!(!plan.folder().exists());

        // And then it works: the next try starts clean and clears the reason.
        plan.install(&scratch.npm(INSTALLS), &mut |_| {}).unwrap();
        assert!(plan.is_ready());
        assert!(!plan.root().join(STATE).exists());
    }

    #[test]
    fn no_npm_is_said_in_words() {
        let scratch = Scratch::new("no-npm");
        let plan = scratch.plan("lock", "source");
        let failure = plan.install(&Npm { program: PathBuf::from("team-hub-no-such-npm"), beside_node: None }, &mut |_| {}).unwrap_err();

        assert!(failure.reason.contains("npm could not be started"), "{failure:?}");
        assert!(failure.reason.contains("nodejs.org"), "{failure:?}");
        assert!(!plan.folder().exists());
    }

    #[test]
    fn a_folder_that_lost_its_marker_is_fetched_again_not_trusted() {
        let scratch = Scratch::new("marker");
        let plan = scratch.plan("lock", "source");

        scratch.write(&format!("data/agent-runtime/{}/node_modules/half", plan.folder().file_name().unwrap().to_string_lossy()), "");
        assert!(!plan.is_complete());

        // The attempt fails here (there is no npm), and takes the half folder with it.
        let _ = plan.install(&Npm { program: PathBuf::from("team-hub-no-such-npm"), beside_node: None }, &mut |_| {});

        assert!(!plan.folder().exists());
    }

    #[test]
    fn npm_errors_are_told_apart() {
        assert!(reason_for("npm error code ENOTFOUND\nnpm error network request to …", Some(1)).contains("could not be reached"));
        assert!(reason_for("npm error code EAI_AGAIN", Some(1)).contains("could not be reached"));
        assert!(reason_for("npm error code E403\nnpm error 403 Forbidden - GET https://registry/…", Some(1)).contains("refused"));
        assert!(reason_for("npm error code E401", Some(1)).contains("refused"));
        assert!(reason_for("npm error code ENOSPC", Some(1)).contains("disk is full"));
        assert!(reason_for("npm error code EACCES", Some(243)).contains("not allowed to write"));
        assert!(reason_for("npm error code EINTEGRITY", Some(1)).contains("lock file"));
        assert_eq!(reason_for("something else", Some(7)), "npm ended with exit code 7.");
        assert_eq!(reason_for("", None), "npm was stopped before it finished.");
    }

    #[test]
    fn npm_is_the_one_beside_the_node_of_the_settings() {
        let scratch = Scratch::new("npm");
        let name = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let node = scratch.write("node22/bin/node", "");

        // On the PATH: so is npm.
        assert_eq!(Npm::for_node("node"), Npm { program: PathBuf::from(name), beside_node: None });
        // By its path, with no npm beside it: npm from the PATH.
        assert_eq!(Npm::for_node(node.to_str().unwrap()), Npm { program: PathBuf::from(name), beside_node: None });

        let beside = scratch.write(&format!("node22/bin/{name}"), "");

        assert_eq!(Npm::for_node(node.to_str().unwrap()), Npm { program: beside, beside_node: Some(scratch.0.join("node22/bin")) });
    }

    #[test]
    fn the_state_is_json_whatever_the_reason_says() {
        assert_eq!(json_text("a \"quoted\" path C:\\x\nnext\ttab\u{1}"), "\"a \\\"quoted\\\" path C:\\\\x\\nnext\\ttab\\u0001\"");
    }
}
