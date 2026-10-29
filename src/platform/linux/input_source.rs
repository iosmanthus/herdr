//! Prefix-mode input source switching for the Linux input method frameworks
//! fcitx5 and ibus.

use std::process::{Command, Stdio};

/// Restores the host input method to its pre-prefix state when dropped.
///
/// Built by [`switch_to_ascii_input_source`] when prefix mode parks the IME in
/// an ASCII-capable state; the client drops it when it leaves the mode, which
/// reverses the switch on a best-effort basis.
#[derive(Debug)]
pub(crate) struct InputSourceRestore {
    undo: Undo,
}

impl Drop for InputSourceRestore {
    fn drop(&mut self) {
        let _ = match &self.undo {
            Undo::Fcitx5Activate => run_ime_capture("fcitx5-remote", &["-o"]),
            Undo::IbusSetEngine(engine) => run_ime_capture("ibus", &["engine", engine.as_str()]),
        };
    }
}

/// How to reverse a prefix-mode ASCII switch, per backend.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Undo {
    /// fcitx5 was active (composing); activate it again.
    Fcitx5Activate,
    /// ibus was on this engine; switch back to it.
    IbusSetEngine(String),
}

/// Host input method framework that herdr can park in ASCII mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImeBackend {
    Fcitx5,
    Ibus,
}

/// Park the host IME in an ASCII-capable state so that prefix-mode command
/// keys reach herdr instead of the IME, and return a guard that restores the
/// previous state on drop.
///
/// Returns `None` when no supported IME is active, it is already in ASCII
/// mode, or its control tool is not available. This is the Linux counterpart
/// of the macOS and Windows switch behind
/// `[experimental] switch_ascii_input_source_in_prefix`.
pub(crate) fn switch_to_ascii_input_source() -> Option<InputSourceRestore> {
    let backend = detect_ime_backend(|key| std::env::var(key).ok())?;
    let undo = plan_ascii_switch(backend, &mut run_ime_capture)?;
    Some(InputSourceRestore { undo })
}

/// Detect the active IME from the standard input method environment variables.
/// `XMODIFIERS` is the authoritative one; the toolkit hints are the fallback.
fn detect_ime_backend(var: impl Fn(&str) -> Option<String>) -> Option<ImeBackend> {
    ["XMODIFIERS", "GTK_IM_MODULE", "QT_IM_MODULE"]
        .into_iter()
        .filter_map(var)
        .find_map(|value| {
            if value.contains("fcitx") {
                Some(ImeBackend::Fcitx5)
            } else if value.contains("ibus") {
                Some(ImeBackend::Ibus)
            } else {
                None
            }
        })
}

/// Query the backend, switch it to ASCII if it is composing, and return how to
/// undo that. `run` executes an IME command and returns its trimmed stdout, or
/// `None` on failure, so that this logic is testable without a live IME.
fn plan_ascii_switch(
    backend: ImeBackend,
    run: &mut dyn FnMut(&str, &[&str]) -> Option<String>,
) -> Option<Undo> {
    match backend {
        // fcitx5 state codes: 0 closed, 1 inactive (already ASCII), 2 active
        // (composing). Deactivate only when it is composing.
        ImeBackend::Fcitx5 => {
            if run("fcitx5-remote", &[])? != "2" {
                return None;
            }
            run("fcitx5-remote", &["-c"]);
            Some(Undo::Fcitx5Activate)
        }
        // ibus has no global toggle; the ASCII equivalent is an `xkb:*`
        // keyboard engine. Save the current engine to restore it later.
        ImeBackend::Ibus => {
            let current = run("ibus", &["engine"])?;
            if current.is_empty() || current.starts_with("xkb:") {
                return None;
            }
            run("ibus", &["engine", "xkb:us::eng"]);
            Some(Undo::IbusSetEngine(current))
        }
    }
}

/// Run an IME control command and return its trimmed stdout, or `None` if it
/// cannot start or exits with an error.
fn run_ime_capture(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn detect_ime_backend_recognizes_fcitx_ibus_and_unknown() {
        assert_eq!(
            detect_ime_backend(env(&[("XMODIFIERS", "@im=fcitx")])),
            Some(ImeBackend::Fcitx5)
        );
        assert_eq!(
            detect_ime_backend(env(&[("XMODIFIERS", "@im=ibus")])),
            Some(ImeBackend::Ibus)
        );
        assert_eq!(
            detect_ime_backend(env(&[("GTK_IM_MODULE", "fcitx")])),
            Some(ImeBackend::Fcitx5)
        );
        assert_eq!(detect_ime_backend(env(&[("XMODIFIERS", "@im=none")])), None);
        assert_eq!(detect_ime_backend(env(&[])), None);
    }

    #[test]
    fn fcitx5_deactivates_only_when_active() {
        let mut calls: Vec<String> = Vec::new();
        let undo = plan_ascii_switch(ImeBackend::Fcitx5, &mut |program, args| {
            calls.push(format!("{program} {}", args.join(" ")));
            let state = if args.is_empty() { "2" } else { "" };
            Some(state.to_owned())
        });
        assert_eq!(undo, Some(Undo::Fcitx5Activate));
        assert!(calls.iter().any(|call| call == "fcitx5-remote -c"));

        let undo = plan_ascii_switch(ImeBackend::Fcitx5, &mut |_, args| {
            let state = if args.is_empty() { "1" } else { "" };
            Some(state.to_owned())
        });
        assert_eq!(undo, None);
    }

    #[test]
    fn ibus_saves_engine_and_switches_to_ascii() {
        let mut calls: Vec<String> = Vec::new();
        let undo = plan_ascii_switch(ImeBackend::Ibus, &mut |program, args| {
            calls.push(format!("{program} {}", args.join(" ")));
            let engine = if args == ["engine"] { "rime" } else { "" };
            Some(engine.to_owned())
        });
        assert_eq!(undo, Some(Undo::IbusSetEngine("rime".to_owned())));
        assert!(calls.iter().any(|call| call == "ibus engine xkb:us::eng"));
    }

    #[test]
    fn ibus_is_noop_when_already_on_xkb_engine() {
        let undo = plan_ascii_switch(ImeBackend::Ibus, &mut |_, args| {
            let engine = if args == ["engine"] {
                "xkb:us::eng"
            } else {
                ""
            };
            Some(engine.to_owned())
        });
        assert_eq!(undo, None);
    }

    #[test]
    fn switch_is_noop_when_backend_query_unavailable() {
        assert_eq!(
            plan_ascii_switch(ImeBackend::Fcitx5, &mut |_, _| None),
            None
        );
    }
}
