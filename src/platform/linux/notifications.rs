//! Linux desktop notifications through libnotify's `notify-send`, with a click
//! action, and raising of the terminal window that hosts this client.

use std::io::{self, BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;

use sha2::{Digest, Sha256};

/// Runs on a waiter thread when the user clicks the notification.
pub(crate) type Callback = Arc<dyn Fn() + Send + Sync>;

/// Key of notifications without a pane. They do not replace one another.
const GENERAL_KEY: &str = "general";

/// The action that the notification daemon invokes on a click on the body.
const DEFAULT_ACTION: &str = "default";

/// Show a notification. A click raises the terminal window of this client.
pub fn show_desktop_notification(title: &str, body: Option<&str>) -> io::Result<bool> {
    show_actionable_desktop_notification(
        title,
        body,
        GENERAL_KEY.to_owned(),
        Arc::new(foreground_desktop_notification_host),
    )
}

/// Show a notification that runs `callback` when the user clicks it. A newer
/// notification with the same `key` replaces this one in daemons that read the
/// stack tag hint (quickshell, dunst).
pub(crate) fn show_actionable_desktop_notification(
    title: &str,
    body: Option<&str>,
    key: String,
    callback: Callback,
) -> io::Result<bool> {
    let has_display =
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    show_with_command(title, body, &key, callback, has_display, |program| {
        Command::new(program)
    })
    .map(|waiter| waiter.is_some())
}

fn show_with_command(
    title: &str,
    body: Option<&str>,
    key: &str,
    callback: Callback,
    has_display: bool,
    mut command: impl FnMut(&str) -> Command,
) -> io::Result<Option<JoinHandle<()>>> {
    if !has_display {
        return Ok(None);
    }
    let mut notify = command("notify-send");
    notify
        .args(notification_args(title, body, key, true))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = match notify.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut fallback = command("notify-send");
    fallback.args(notification_args(title, body, key, false));
    let waiter = std::thread::Builder::new()
        .name("herdr-notify-wait".into())
        .spawn(move || wait_for_click(child, &*callback, fallback))?;
    Ok(Some(waiter))
}

fn notification_args(title: &str, body: Option<&str>, key: &str, action: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "--app-name",
        "Herdr",
        "--icon",
        "herdr",
        "--hint",
        "string:desktop-entry:herdr",
    ]
    .map(str::to_owned)
    .into();
    if key != GENERAL_KEY {
        args.push("--hint".to_owned());
        args.push(format!("string:x-dunst-stack-tag:herdr-{}", stack_tag(key)));
    }
    if action {
        args.push("--action".to_owned());
        args.push(format!("{DEFAULT_ACTION}=Open"));
    }
    args.push("--".to_owned());
    args.push(title.to_owned());
    if let Some(body) = body.filter(|body| !body.is_empty()) {
        args.push(body.to_owned());
    }
    args
}

/// A short, stable tag for a notification key. The key itself is JSON.
fn stack_tag(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Wait until the notification closes, then reap `notify-send`.
fn wait_for_click(mut child: Child, on_click: &(dyn Fn() + Send + Sync), mut fallback: Command) {
    let output = child
        .stdout
        .take()
        .is_some_and(|stdout| read_actions(stdout, on_click));
    let succeeded = child.wait().is_ok_and(|status| status.success());
    if !succeeded && !output {
        // libnotify before 0.8 has no --action: show the notification without one.
        run_quietly(&mut fallback);
    }
}

/// Read the output of `notify-send --action` until EOF. It prints the name of
/// the invoked action, and exits when the daemon closes the notification. The
/// callback runs when the `default` line arrives, not at EOF: a daemon can
/// keep a notification open after its action runs. Returns whether any line
/// came.
fn read_actions(output: impl Read, on_click: &(dyn Fn() + Send + Sync)) -> bool {
    let mut any_line = false;
    let mut clicked = false;
    for line in BufReader::new(output).lines() {
        let Ok(line) = line else {
            break;
        };
        any_line = true;
        if !clicked && line.trim() == DEFAULT_ACTION {
            clicked = true;
            on_click();
        }
    }
    any_line
}

/// Raise the terminal window that hosts this client. Best effort.
///
/// Under niri the window comes from niri's IPC. Everywhere else this uses the
/// X11 window id in `$WINDOWID`, which kitty and most X terminals set.
/// Wayland-native terminals do not set it, so outside niri on Wayland nothing
/// is raised.
pub(crate) fn foreground_desktop_notification_host() {
    if std::env::var_os("NIRI_SOCKET").is_some() && raise_niri_host_window() {
        return;
    }
    if let Some(window_id) = x11_window_id(std::env::var("WINDOWID").ok()) {
        raise_x11_window(&window_id);
    }
}

/// niri reports each window with the pid of its client, so the window that
/// hosts this herdr client is the one of our nearest ancestor: normally the
/// terminal emulator directly above us.
fn raise_niri_host_window() -> bool {
    let Ok(output) = Command::new("niri")
        .args(["msg", "--json", "windows"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Some(windows) = parse_niri_window_owners(&output.stdout) else {
        return false;
    };
    let ancestors = process_ancestors(std::process::id());
    let Some(window_id) = host_window_id(&windows, &ancestors) else {
        return false;
    };
    run_quietly(
        Command::new("niri")
            .args(["msg", "action", "focus-window", "--id"])
            .arg(window_id.to_string()),
    )
}

/// `(window id, owner pid)` for each window in `niri msg --json windows`.
/// Windows without a pid are skipped.
fn parse_niri_window_owners(json: &[u8]) -> Option<Vec<(u64, u32)>> {
    #[derive(serde::Deserialize)]
    struct NiriWindow {
        id: u64,
        pid: Option<i32>,
    }

    let windows: Vec<NiriWindow> = serde_json::from_slice(json).ok()?;
    Some(
        windows
            .into_iter()
            .filter_map(|window| Some((window.id, u32::try_from(window.pid?).ok()?)))
            .collect(),
    )
}

/// `pid` followed by its parents, nearest first, stopping before init.
fn process_ancestors(pid: u32) -> Vec<u32> {
    const MAX_DEPTH: usize = 64;

    let mut chain = Vec::new();
    let mut current = Some(pid);
    while let Some(pid) = current {
        if pid <= 1 || chain.len() >= MAX_DEPTH || chain.contains(&pid) {
            break;
        }
        chain.push(pid);
        current = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| parent_pid_from_stat(&stat));
    }
    chain
}

fn parent_pid_from_stat(stat: &str) -> Option<u32> {
    // "pid (comm) state ppid ...": comm can contain spaces and parentheses, so
    // split after the last ')'.
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// The window of the nearest ancestor. One terminal process can have more than
/// one window; they then share a pid and the first one listed wins.
fn host_window_id(windows: &[(u64, u32)], ancestors: &[u32]) -> Option<u64> {
    ancestors.iter().find_map(|pid| {
        windows
            .iter()
            .find(|(_, owner)| owner == pid)
            .map(|(id, _)| *id)
    })
}

fn x11_window_id(value: Option<String>) -> Option<String> {
    value.filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn raise_x11_window(window_id: &str) -> bool {
    run_quietly(Command::new("xdotool").args(["windowactivate", window_id]))
        || run_quietly(Command::new("wmctrl").args(["-ia", window_id]))
}

fn run_quietly(command: &mut Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn lines(args: &[String]) -> String {
        args.iter().map(|arg| format!("{arg}\n")).collect()
    }

    /// A fake `notify-send`: a shell script, with the real arguments as `$@`.
    /// It uses an absolute shell and only shell builtins, because other tests
    /// in this process change `PATH` while they run.
    fn fake(script: &str) -> impl FnMut(&str) -> Command + '_ {
        move |program| {
            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c").arg(script).arg(program);
            cmd
        }
    }

    fn channel_callback() -> (Callback, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        (
            Arc::new(move || {
                let _ = tx.lock().unwrap().send(());
            }),
            rx,
        )
    }

    #[test]
    fn general_notification_has_icon_and_action_without_stack_tag() {
        assert_eq!(
            lines(&notification_args(
                "-danger",
                Some("body"),
                GENERAL_KEY,
                true
            )),
            "--app-name\nHerdr\n--icon\nherdr\n--hint\nstring:desktop-entry:herdr\n\
             --action\ndefault=Open\n--\n-danger\nbody\n"
        );
    }

    #[test]
    fn pane_notification_has_stable_stack_tag() {
        let key = r#"["local","boot","w1:p1"]"#;
        let args = notification_args("done", None, key, false);
        let tag = format!("string:x-dunst-stack-tag:herdr-{}", stack_tag(key));
        assert_eq!(stack_tag(key).len(), 16);
        assert_eq!(stack_tag(key), stack_tag(key));
        assert_ne!(stack_tag(key), stack_tag(r#"["local","boot","w1:p2"]"#));
        assert!(args.windows(2).any(|pair| pair == ["--hint", tag.as_str()]));
        assert!(!args.iter().any(|arg| arg == "--action"));
        assert_eq!(args.last().map(String::as_str), Some("done"));
    }

    #[test]
    fn empty_body_is_left_out() {
        let args = notification_args("done", Some(""), GENERAL_KEY, false);
        assert_eq!(args.last().map(String::as_str), Some("done"));
    }

    #[test]
    fn no_display_shows_nothing() {
        let (callback, _rx) = channel_callback();
        let waiter =
            show_with_command("t", None, GENERAL_KEY, callback, false, fake("exit 0")).unwrap();
        assert!(waiter.is_none());
    }

    #[test]
    fn missing_notify_send_shows_nothing() {
        let (callback, _rx) = channel_callback();
        let waiter = show_with_command("t", None, GENERAL_KEY, callback, true, |_| {
            Command::new("/nonexistent/herdr-notify-send")
        })
        .unwrap();
        assert!(waiter.is_none());
    }

    #[test]
    fn default_action_line_calls_back() {
        let (callback, rx) = channel_callback();
        let waiter = show_with_command(
            "t",
            None,
            GENERAL_KEY,
            callback,
            true,
            fake("printf 'default\\n'"),
        )
        .unwrap()
        .unwrap();
        waiter.join().unwrap();
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn closed_without_action_does_not_call_back() {
        let (callback, rx) = channel_callback();
        let waiter = show_with_command("t", None, GENERAL_KEY, callback, true, fake("exit 0"))
            .unwrap()
            .unwrap();
        waiter.join().unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn callback_runs_before_the_notification_closes() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let (callback, rx) = channel_callback();
        let reading = std::thread::spawn(move || read_actions(reader, &*callback));
        std::io::Write::write_all(&mut writer, b"default\n").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_secs(3)).is_ok(),
            "the click must not wait for the notification to close"
        );
        drop(writer);
        assert!(reading.join().unwrap());
    }

    #[test]
    fn only_the_first_default_line_calls_back() {
        let (callback, rx) = channel_callback();
        assert!(read_actions(&b"other\ndefault\ndefault\n"[..], &*callback));
        assert_eq!(rx.try_iter().count(), 1);
        assert!(!read_actions(&b""[..], &*callback));
    }

    #[test]
    fn failure_without_output_shows_the_notification_without_action() {
        let dir =
            std::env::temp_dir().join(format!("herdr-notify-fallback-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let marker = dir.display().to_string();
        let script = format!(
            "case \" $* \" in *' --action '*) exit 1;; *) printf '%s\\n' \"$@\" > '{marker}';; esac"
        );
        let (callback, rx) = channel_callback();
        let waiter = show_with_command("t", None, GENERAL_KEY, callback, true, fake(&script))
            .unwrap()
            .unwrap();
        waiter.join().unwrap();
        let fallback_args = std::fs::read_to_string(&dir).expect("fallback ran");
        let _ = std::fs::remove_file(&dir);
        assert!(rx.try_recv().is_err());
        assert_eq!(
            fallback_args,
            lines(&notification_args("t", None, GENERAL_KEY, false))
        );
    }

    #[test]
    fn parent_pid_from_stat_skips_comm_with_spaces_and_parens() {
        let stat = "4242 (kitty (main) win) S 1717 4242 4242 0 -1 4194560 1234";
        assert_eq!(parent_pid_from_stat(stat), Some(1717));
        assert_eq!(parent_pid_from_stat("garbage"), None);
    }

    #[test]
    fn parse_niri_window_owners_keeps_windows_with_a_pid() {
        let json = br#"[
            {"id": 2, "title": "herdr", "app_id": "kitty", "pid": 370098,
             "workspace_id": 1, "is_focused": true, "layout": {}},
            {"id": 7, "title": "wayland-only", "app_id": null, "pid": null},
            {"id": 9, "title": "Bitwarden", "app_id": "Bitwarden", "pid": 382001}
        ]"#;
        assert_eq!(
            parse_niri_window_owners(json),
            Some(vec![(2, 370098), (9, 382001)])
        );
        assert_eq!(parse_niri_window_owners(b"not json"), None);
    }

    #[test]
    fn host_window_id_prefers_the_nearest_ancestor() {
        let windows = [(2, 100), (5, 300), (6, 300)];
        assert_eq!(host_window_id(&windows, &[400, 300, 100]), Some(5));
        assert_eq!(host_window_id(&windows, &[400, 100]), Some(2));
        assert_eq!(host_window_id(&windows, &[400, 500]), None);
    }

    #[test]
    fn process_ancestors_starts_with_self_and_walks_up() {
        let chain = process_ancestors(std::process::id());
        assert_eq!(chain.first(), Some(&std::process::id()));
        assert!(!chain.contains(&1));
    }

    #[test]
    fn x11_window_id_must_be_numeric() {
        assert_eq!(
            x11_window_id(Some("12345678".into())),
            Some("12345678".into())
        );
        assert_eq!(x11_window_id(Some("not-a-number".into())), None);
        assert_eq!(x11_window_id(Some(String::new())), None);
        assert_eq!(x11_window_id(None), None);
    }
}
