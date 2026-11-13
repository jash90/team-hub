//! What an installed app has to find out about the computer before it can start an agent:
//! the `PATH` a terminal would have, whether the Node on it can run the helper, and the
//! helper's script in a form Node accepts.
//!
//! Nothing here needs Tauri, so `rustc --test src/environment.rs` runs its tests alone.

#[cfg(unix)]
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The helper is TypeScript run by Node itself, which Node does without a flag from here on.
const NODE_22: (u32, u32) = (22, 18);
const NODE_23: (u32, u32) = (23, 6);

#[cfg(unix)]
const MARK: &str = "__TEAM_HUB_PATH__";

/// Runs a short command and gives back what it printed. `complete` says when enough was
/// printed, so that something the command left running with the pipe open is not waited for.
fn printed_by(mut command: Command, limit: Duration, complete: fn(&str) -> bool) -> std::io::Result<Option<String>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let Some(mut stdout) = child.stdout.take() else { return Ok(None) };
    let (found, printed) = mpsc::channel();

    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];

        loop {
            let read = stdout.read(&mut chunk).unwrap_or(0);

            bytes.extend_from_slice(&chunk[..read]);

            if read == 0 || complete(&String::from_utf8_lossy(&bytes)) {
                break;
            }
        }

        let _ = found.send(String::from_utf8_lossy(&bytes).into_owned());
    });

    let deadline = Instant::now() + limit;
    let text = printed.recv_timeout(limit).ok();

    // Whatever happened, the command itself is not left behind.
    while text.is_some() && Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(10));
    }

    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
        let _ = child.wait();
    }

    Ok(text)
}

/// `one:two` and more folders as one `PATH`: the first list wins the order, nothing twice.
#[cfg(unix)]
fn merged(lists: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = Vec::new();

    for folder in lists.iter().flatten() {
        if !folder.as_os_str().is_empty() && !all.contains(folder) {
            all.push(folder.clone());
        }
    }

    all
}

/// What a shell printed between the two marks, whatever its start-up files printed around it.
#[cfg(unix)]
fn between_marks(printed: &str) -> Option<&str> {
    let start = printed.find(MARK)? + MARK.len();
    let length = printed[start..].find(MARK)?;

    Some(&printed[start..start + length]).filter(|path| !path.trim().is_empty())
}

/// The `PATH` of the person's own shell, as a terminal window would have it.
#[cfg(unix)]
fn shell_path(shell: &Path) -> Option<OsString> {
    // Shells known to take `-ilc` and to print `$PATH` joined with colons.
    let known = ["bash", "zsh", "sh", "dash", "ksh", "fish"];
    let name = shell.file_name()?.to_str()?;

    if !known.contains(&name) {
        return None;
    }

    let mut command = Command::new(shell);

    // Login and interactive: Homebrew is put on the PATH by the first kind of start-up file,
    // version managers such as nvm by the second.
    command.args(["-ilc", &format!("printf '%s%s%s' {MARK} \"$PATH\" {MARK}")]).env("TEAM_HUB_RESOLVING_PATH", "1");

    let complete = |printed: &str| between_marks(printed).is_some();
    let printed = printed_by(command, Duration::from_secs(5), complete).ok()??;

    between_marks(&printed).map(OsString::from)
}

/// Where Node, git and `gh` usually are when no start-up file says so.
#[cfg(unix)]
fn usual_folders() -> Vec<PathBuf> {
    let mut folders = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];

    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
        folders.push(Path::new(&home).join(".local/bin"));
        folders.push(Path::new(&home).join(".volta/bin"));
    }

    folders.into_iter().filter(|folder| folder.is_dir()).collect()
}

/// Gives this process the `PATH` of the person's shell. An app started from the Dock, the
/// Finder or a launcher gets the system's short one, on which a Node from Homebrew or a
/// version manager is not found, and neither are the tools an agent's shell is asked to run.
/// Started from a terminal there is nothing to do.
///
/// Call it first thing in `main`: it changes the environment, and nothing else may be
/// running yet.
#[cfg(unix)]
pub fn adopt_shell_path() {
    use std::io::IsTerminal;

    if std::io::stdin().is_terminal() {
        return;
    }

    let current = std::env::var_os("PATH").unwrap_or_default();
    let shell = std::env::var_os("SHELL").filter(|shell| !shell.is_empty()).map(PathBuf::from);
    let shell = shell.or_else(|| ["/bin/zsh", "/bin/bash"].iter().map(PathBuf::from).find(|shell| shell.exists()));
    let from_shell = shell.and_then(|shell| shell_path(&shell)).unwrap_or_default();
    let folders = merged(&[std::env::split_paths(&from_shell).collect(), std::env::split_paths(&current).collect(), usual_folders()]);

    if let Ok(path) = std::env::join_paths(folders) {
        if path != current {
            std::env::set_var("PATH", path);
        }
    }
}

/// Windows gives a window the same `PATH` as a terminal.
#[cfg(not(unix))]
pub fn adopt_shell_path() {}

/// `\\?\C:\Users\me\…` as `C:\Users\me\…`. Tauri gives the app's own folder in the first
/// form, in which Windows takes every character literally (a `/` is no separator) and which
/// Node does not run a script from.
pub fn plain_path(path: &Path) -> PathBuf {
    const LONGEST_PLAIN: usize = 247;

    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') && rest.len() <= LONGEST_PLAIN => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// `v22.23.4` → `(22, 23)`.
fn node_version(printed: &str) -> Option<(u32, u32)> {
    let mut numbers = printed.trim().strip_prefix('v')?.split(|character: char| !character.is_ascii_digit());
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;

    Some((major, minor))
}

fn runs_the_helper((major, minor): (u32, u32)) -> bool {
    match major {
        22 => minor >= NODE_22.1,
        23 => minor >= NODE_23.1,
        _ => major > NODE_23.0,
    }
}

/// What is wrong with the Node the settings name, in words for a person; `None` when it can
/// run the helper, and also when that could not be found out.
pub fn node_problem(node: &str) -> Option<String> {
    let mut command = Command::new(node);

    command.arg("--version");

    // A window has no console; without this Windows opens one for the moment Node runs.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(0x0800_0000);
    }

    let advice = format!(
        "Agent threads need Node.js {}.{} or newer. Install it from https://nodejs.org, or put the full path of the node program \
         in Settings → Node command (in a terminal: `which node`, on Windows `where node`), then start team-hub again. \
         The rest of the app works without it.",
        NODE_22.0, NODE_22.1
    );

    match printed_by(command, Duration::from_secs(10), |_| false) {
        Err(error) => Some(format!("Node.js was not found: \"{node}\" could not be started ({error}).\n\n{advice}")),
        Ok(printed) => {
            let printed = printed?;
            let version = node_version(&printed)?;

            (!runs_the_helper(version)).then(|| format!("The Node.js found is too old: \"{node}\" is {}.\n\n{advice}", printed.trim()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_versions_that_run_typescript_themselves() {
        assert_eq!(node_version("v22.23.4\n"), Some((22, 23)));
        assert_eq!(node_version("v25.0.0-nightly20260101abcdef\n"), Some((25, 0)));
        assert_eq!(node_version("command not found"), None);
        assert_eq!(node_version(""), None);

        assert!(!runs_the_helper((20, 19)));
        assert!(!runs_the_helper((22, 17)));
        assert!(runs_the_helper((22, 18)));
        assert!(!runs_the_helper((23, 5)));
        assert!(runs_the_helper((23, 6)));
        assert!(runs_the_helper((24, 0)));
    }

    #[test]
    fn a_missing_node_is_named_and_a_present_one_is_measured() {
        let missing = node_problem("team-hub-no-such-node").expect("a program that does not exist is a problem");

        assert!(missing.contains("was not found"), "{missing}");
        assert!(missing.contains("team-hub-no-such-node"), "{missing}");
        assert!(missing.contains("Settings → Node command"), "{missing}");

        // Whatever Node this machine has, the answer follows its version; no Node, no test.
        let Ok(Some(printed)) = printed_by(
            {
                let mut command = Command::new("node");
                command.arg("--version");
                command
            },
            Duration::from_secs(10),
            |_| false,
        ) else {
            return;
        };
        let fine = node_version(&printed).is_some_and(runs_the_helper);

        assert_eq!(node_problem("node").is_none(), fine, "node is {printed}");
    }

    #[cfg(unix)]
    #[test]
    fn something_that_is_not_node_is_not_called_too_old() {
        // `true --version` prints nothing a version can be read from: unknown, so no complaint.
        assert_eq!(node_problem("true"), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_old_node_is_told_apart() {
        let folder = std::env::temp_dir().join(format!("team-hub-old-node-{}", std::process::id()));
        let script = folder.join("node");

        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&script, "#!/bin/sh\necho v20.11.1\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let problem = node_problem(script.to_str().unwrap()).expect("Node 20 cannot run the helper");

        assert!(problem.contains("too old"), "{problem}");
        assert!(problem.contains("v20.11.1"), "{problem}");

        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_long_form_of_a_windows_path_is_made_plain() {
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\Users\me\AppData\Local\team-hub\sidecar\src\main.ts")),
            Path::new(r"C:\Users\me\AppData\Local\team-hub\sidecar\src\main.ts")
        );
        // A share, and anything else, stays as it is.
        assert_eq!(plain_path(Path::new(r"\\?\UNC\server\share\main.ts")), Path::new(r"\\?\UNC\server\share\main.ts"));
        assert_eq!(plain_path(Path::new("/usr/lib/team-hub/sidecar/src/main.ts")), Path::new("/usr/lib/team-hub/sidecar/src/main.ts"));

        // Too long for the plain form: only the long one works, so it is kept.
        let long = format!(r"\\?\C:\{}", "a".repeat(300));

        assert_eq!(plain_path(Path::new(&long)), Path::new(&long));
    }

    #[cfg(unix)]
    #[test]
    fn folders_are_merged_in_order_and_once() {
        let list = |text: &str| text.split(':').map(PathBuf::from).collect::<Vec<_>>();
        let all = merged(&[list("/shell/bin:/usr/bin"), list("/usr/bin:/bin:"), list("/opt/homebrew/bin:/shell/bin")]);

        assert_eq!(all, list("/shell/bin:/usr/bin:/bin:/opt/homebrew/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn the_path_is_read_between_the_marks() {
        let noisy = format!("Welcome back!\n{MARK}/opt/homebrew/bin:/usr/bin{MARK}bye\n");

        assert_eq!(between_marks(&noisy), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(between_marks(&format!("{MARK}/usr/bin")), None);
        assert_eq!(between_marks(&format!("{MARK}{MARK}")), None);
        assert_eq!(between_marks("no marks"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_is_asked_for_its_path() {
        // Every Unix has `sh`; it is asked the same way the person's shell is.
        let path = shell_path(Path::new("/bin/sh")).expect("sh prints its PATH");

        assert!(!path.is_empty());
        assert!(std::env::split_paths(&path).count() >= 1);
        assert_eq!(shell_path(Path::new("/usr/bin/some-other-shell")), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_command_that_keeps_the_pipe_open_is_not_waited_for() {
        let mut command = Command::new("/bin/sh");

        // Prints, then leaves something running that holds the output open.
        command.args(["-c", "printf done; sleep 30 &"]);

        let started = Instant::now();
        let printed = printed_by(command, Duration::from_secs(5), |printed| printed.contains("done")).unwrap();

        assert_eq!(printed.as_deref(), Some("done"));
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[cfg(unix)]
    #[test]
    fn a_command_that_never_answers_is_given_up_on() {
        let mut command = Command::new("/bin/sh");

        command.args(["-c", "sleep 30"]);

        let started = Instant::now();
        let printed = printed_by(command, Duration::from_millis(300), |_| false).unwrap();

        assert_eq!(printed, None);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
