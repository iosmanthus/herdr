use std::path::Path;

use serde::{Deserialize, Serialize};

const MAX_SESSION_ID_LEN: usize = 512;
const MAX_SESSION_PATH_LEN: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionRef {
    pub kind: AgentSessionRefKind,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionRefKind {
    Id,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResumePlan {
    pub agent: String,
    pub argv: Vec<String>,
    pub dedupe_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAgentSession {
    pub source: String,
    pub agent: String,
    pub session_ref: AgentSessionRef,
}

impl AgentSessionRef {
    pub fn id(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_session_id(&value).then_some(Self {
            kind: AgentSessionRefKind::Id,
            value,
        })
    }

    pub fn path(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_session_path(&value).then_some(Self {
            kind: AgentSessionRefKind::Path,
            value,
        })
    }
}

pub fn session_ref_from_report(
    source: &str,
    agent: &str,
    agent_session_id: Option<String>,
    _agent_session_path: Option<String>,
) -> Option<AgentSessionRef> {
    if !is_official_agent_source(source, agent) {
        return None;
    }

    if agent == "pi" {
        return _agent_session_path
            .and_then(AgentSessionRef::path)
            .or_else(|| agent_session_id.and_then(AgentSessionRef::id));
    }

    agent_session_id.and_then(AgentSessionRef::id)
}

pub fn is_reserved_native_state_source(source: &str, agent: &str) -> bool {
    matches!(
        (source, agent),
        ("herdr:claude", "claude")
            | ("herdr:codex", "codex")
            | ("herdr:copilot", "copilot")
            | ("herdr:droid", "droid")
            | ("herdr:qodercli", "qodercli")
            | ("herdr:cursor", "cursor")
    )
}

pub fn session_ref_from_snapshot(
    source: &str,
    agent: &str,
    kind: AgentSessionRefKind,
    value: &str,
) -> Option<PersistedAgentSession> {
    if !is_official_agent_source(source, agent) {
        return None;
    }
    let session_ref = match (agent, kind) {
        ("pi", AgentSessionRefKind::Path) => AgentSessionRef::path(value)?,
        (_, AgentSessionRefKind::Id) => AgentSessionRef::id(value)?,
        _ => return None,
    };
    Some(PersistedAgentSession {
        source: source.to_string(),
        agent: agent.to_string(),
        session_ref,
    })
}

/// Convenience wrapper exercising the canonical resume command without any
/// recorded launch arguments. Production code calls [`plan_with_launch_argv`]
/// directly, so this is only needed by tests.
#[cfg(test)]
fn plan(source: &str, agent: &str, session_ref: &AgentSessionRef) -> Option<AgentResumePlan> {
    plan_with_launch_argv(source, agent, session_ref, None)
}

/// Build a resume plan, optionally restoring the pane's original launch command
/// verbatim.
///
/// When `launch_argv` records the exact command herdr used to start the agent,
/// resume it as-is and append the agent's resume reference. This preserves
/// launch flags such as `--dangerously-skip-permissions` or `--model` across a
/// restart. Without a recorded launch command (e.g. agents detected from a
/// hand-typed shell), fall back to the canonical program name plus the resume
/// reference, matching the historical behavior.
pub fn plan_with_launch_argv(
    source: &str,
    agent: &str,
    session_ref: &AgentSessionRef,
    launch_argv: Option<&[String]>,
) -> Option<AgentResumePlan> {
    if !is_official_agent_source(source, agent) {
        return None;
    }

    let (canonical_program, resume_args) = resume_components(source, agent, session_ref)?;

    // When replaying a recorded launch command, strip any session selector it
    // already carries (e.g. `--resume <id>`, `--continue`, codex's `resume`
    // subcommand) before appending our own. This guarantees the replayed
    // command never ends up with two conflicting selectors.
    let mut argv = match launch_argv {
        Some(launch) if !launch.is_empty() => {
            let mut argv = Vec::with_capacity(launch.len() + resume_args.len());
            argv.push(launch[0].clone());
            argv.extend(strip_session_selectors(agent, &launch[1..]));
            argv
        }
        _ => vec![canonical_program.to_string()],
    };
    argv.extend(resume_args);

    Some(AgentResumePlan {
        agent: agent.to_string(),
        argv,
        dedupe_key: dedupe_key(source, agent, session_ref),
    })
}

/// Map a supported `(source, agent, session ref)` to its canonical program name
/// and the trailing arguments that select the saved session.
fn resume_components(
    source: &str,
    agent: &str,
    session_ref: &AgentSessionRef,
) -> Option<(&'static str, Vec<String>)> {
    let value = &session_ref.value;
    let components = match (source, agent, session_ref.kind) {
        ("herdr:claude", "claude", AgentSessionRefKind::Id) => {
            ("claude", vec!["--resume".into(), value.clone()])
        }
        ("herdr:codex", "codex", AgentSessionRefKind::Id) => {
            ("codex", vec!["resume".into(), value.clone()])
        }
        ("herdr:copilot", "copilot", AgentSessionRefKind::Id) => {
            ("copilot", vec![format!("--resume={value}")])
        }
        ("herdr:droid", "droid", AgentSessionRefKind::Id) => {
            ("droid", vec!["--resume".into(), value.clone()])
        }
        ("herdr:kimi", "kimi", AgentSessionRefKind::Id) => {
            ("kimi", vec!["--session".into(), value.clone()])
        }
        ("herdr:pi", "pi", AgentSessionRefKind::Path | AgentSessionRefKind::Id) => {
            ("pi", vec!["--session".into(), value.clone()])
        }
        ("herdr:hermes", "hermes", AgentSessionRefKind::Id) => {
            ("hermes", vec!["--resume".into(), value.clone()])
        }
        ("herdr:opencode", "opencode", AgentSessionRefKind::Id) => {
            ("opencode", vec!["--session".into(), value.clone()])
        }
        ("herdr:qodercli", "qodercli", AgentSessionRefKind::Id) => {
            ("qodercli", vec!["--resume".into(), value.clone()])
        }
        ("herdr:kilo", "kilo", AgentSessionRefKind::Id) => {
            ("kilo", vec!["--session".into(), value.clone()])
        }
        ("herdr:cursor", "cursor", AgentSessionRefKind::Id) => {
            ("cursor-agent", vec!["--resume".into(), value.clone()])
        }
        _ => return None,
    };
    Some(components)
}

/// Session-selecting tokens an agent's CLI understands, so a recorded launch
/// command can be replayed without colliding with the resume reference we
/// append. Mirrors the selectors emitted by [`resume_components`].
struct SessionSelectors {
    /// Flags that take a following value (`--resume <id>`); also matched in
    /// `--flag=value` form. The value token is dropped too when present.
    value_flags: &'static [&'static str],
    /// Boolean flags with no value (`--continue`); also matched as `--flag=...`.
    bool_flags: &'static [&'static str],
    /// A bare positional subcommand (`codex resume <id>`); the following value
    /// token is dropped too when present.
    subcommand: Option<&'static str>,
}

fn session_selectors(agent: &str) -> SessionSelectors {
    match agent {
        // claude exposes both `--resume`/`-r` (specific or picker) and
        // `--continue`/`-c` (latest conversation).
        "claude" => SessionSelectors {
            value_flags: &["--resume", "-r"],
            bool_flags: &["--continue", "-c"],
            subcommand: None,
        },
        // codex selects via the `resume` subcommand; its `-c`/`--config`
        // override is unrelated and must survive.
        "codex" => SessionSelectors {
            value_flags: &[],
            bool_flags: &[],
            subcommand: Some("resume"),
        },
        "copilot" | "droid" | "hermes" | "qodercli" | "cursor" => SessionSelectors {
            value_flags: &["--resume"],
            bool_flags: &[],
            subcommand: None,
        },
        "kimi" | "pi" | "opencode" | "kilo" => SessionSelectors {
            value_flags: &["--session"],
            bool_flags: &[],
            subcommand: None,
        },
        _ => SessionSelectors {
            value_flags: &[],
            bool_flags: &[],
            subcommand: None,
        },
    }
}

/// Drop any pre-existing session selector from a recorded launch command's
/// arguments (everything after the program name).
fn strip_session_selectors(agent: &str, args: &[String]) -> Vec<String> {
    let selectors = session_selectors(agent);
    let eq_form = |flag: &&str, token: &str| -> bool {
        flag.starts_with("--")
            && token
                .strip_prefix(*flag)
                .is_some_and(|rest| rest.starts_with('='))
    };

    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut subcommand_dropped = false;
    let mut i = 0;
    while i < args.len() {
        let token = args[i].as_str();

        let is_value_flag = selectors.value_flags.contains(&token);
        let is_value_flag_eq = selectors
            .value_flags
            .iter()
            .any(|flag| eq_form(flag, token));
        let is_bool_flag = selectors
            .bool_flags
            .iter()
            .any(|flag| token == *flag || eq_form(flag, token));
        let is_subcommand =
            !subcommand_dropped && selectors.subcommand.is_some_and(|sub| token == sub);

        if is_value_flag || is_subcommand {
            subcommand_dropped |= is_subcommand;
            i += 1;
            // Drop the following value when it isn't itself another flag.
            if i < args.len() && !args[i].starts_with('-') {
                i += 1;
            }
            continue;
        }
        if is_value_flag_eq || is_bool_flag {
            i += 1;
            continue;
        }

        out.push(args[i].clone());
        i += 1;
    }
    out
}

pub fn dedupe_key(source: &str, agent: &str, session_ref: &AgentSessionRef) -> String {
    format!(
        "{source}\u{0}{agent}\u{0}{:?}\u{0}{}",
        session_ref.kind, session_ref.value
    )
}

fn is_official_agent_source(source: &str, agent: &str) -> bool {
    matches!(
        (source, agent),
        ("herdr:claude", "claude")
            | ("herdr:codex", "codex")
            | ("herdr:copilot", "copilot")
            | ("herdr:droid", "droid")
            | ("herdr:kimi", "kimi")
            | ("herdr:pi", "pi")
            | ("herdr:hermes", "hermes")
            | ("herdr:opencode", "opencode")
            | ("herdr:qodercli", "qodercli")
            | ("herdr:kilo", "kilo")
            | ("herdr:cursor", "cursor")
    )
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SESSION_ID_LEN && !value.chars().any(char::is_control)
}

fn valid_session_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_PATH_LEN
        && !value.chars().any(char::is_control)
        && Path::new(value).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute_test_path(name: &str) -> String {
        std::env::current_dir()
            .unwrap()
            .join(name)
            .display()
            .to_string()
    }

    #[test]
    fn native_state_reservation_excludes_full_lifecycle_sources() {
        assert!(is_reserved_native_state_source("herdr:claude", "claude"));
        assert!(is_reserved_native_state_source("herdr:codex", "codex"));
        assert!(!is_reserved_native_state_source("herdr:kimi", "kimi"));
        assert!(!is_reserved_native_state_source(
            "herdr:opencode",
            "opencode"
        ));
    }

    #[test]
    fn planner_allows_supported_agents() {
        let pi_session = absolute_test_path("pi-session.jsonl");
        assert_eq!(
            plan(
                "herdr:claude",
                "claude",
                &AgentSessionRef::id("claude-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["claude", "--resume", "claude-session"]
        );
        assert_eq!(
            plan(
                "herdr:codex",
                "codex",
                &AgentSessionRef::id("codex-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["codex", "resume", "codex-session"]
        );
        assert_eq!(
            plan(
                "herdr:copilot",
                "copilot",
                &AgentSessionRef::id("copilot-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["copilot", "--resume=copilot-session"]
        );
        assert_eq!(
            plan(
                "herdr:droid",
                "droid",
                &AgentSessionRef::id("droid-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["droid", "--resume", "droid-session"]
        );
        assert_eq!(
            plan(
                "herdr:kimi",
                "kimi",
                &AgentSessionRef::id("kimi-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["kimi", "--session", "kimi-session"]
        );
        assert_eq!(
            plan(
                "herdr:pi",
                "pi",
                &AgentSessionRef::path(&pi_session).unwrap()
            )
            .unwrap()
            .argv,
            vec!["pi", "--session", pi_session.as_str()]
        );
        assert_eq!(
            plan(
                "herdr:hermes",
                "hermes",
                &AgentSessionRef::id("hermes-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["hermes", "--resume", "hermes-session"]
        );
        assert_eq!(
            plan(
                "herdr:opencode",
                "opencode",
                &AgentSessionRef::id("opencode-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["opencode", "--session", "opencode-session"]
        );
        assert_eq!(
            plan(
                "herdr:qodercli",
                "qodercli",
                &AgentSessionRef::id("qoder-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["qodercli", "--resume", "qoder-session"]
        );
        assert_eq!(
            plan(
                "herdr:kilo",
                "kilo",
                &AgentSessionRef::id("kilo-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["kilo", "--session", "kilo-session"]
        );
        assert_eq!(
            plan(
                "herdr:cursor",
                "cursor",
                &AgentSessionRef::id("cursor-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["cursor-agent", "--resume", "cursor-session"]
        );
    }

    #[test]
    fn planner_rejects_custom_and_unsupported_path_refs() {
        let claude_session = absolute_test_path("claude-session");
        assert!(plan(
            "custom:claude",
            "claude",
            &AgentSessionRef::id("session").unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:claude",
            "claude",
            &AgentSessionRef::path(&claude_session).unwrap()
        )
        .is_none());
    }

    #[test]
    fn report_ref_prefers_pi_path_and_validates_values() {
        let pi_session = absolute_test_path("pi-session.jsonl");
        let claude_session = absolute_test_path("claude-session");
        let copilot_session = absolute_test_path("copilot-session");
        let session_ref = session_ref_from_report(
            "herdr:pi",
            "pi",
            Some("pi-id".into()),
            Some(pi_session.clone()),
        )
        .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Path);
        assert_eq!(session_ref.value, pi_session);

        assert!(session_ref_from_report("herdr:pi", "pi", Some("bad\nid".into()), None).is_none());
        assert!(
            session_ref_from_report("herdr:pi", "pi", None, Some("relative.jsonl".into()))
                .is_none()
        );
        assert!(session_ref_from_report("custom:pi", "pi", Some("pi-id".into()), None).is_none());
        assert!(
            session_ref_from_report("herdr:claude", "claude", None, Some(claude_session)).is_none()
        );

        let session_ref =
            session_ref_from_report("herdr:copilot", "copilot", Some("copilot-id".into()), None)
                .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "copilot-id");
        assert!(
            session_ref_from_report("herdr:copilot", "copilot", None, Some(copilot_session))
                .is_none()
        );

        let session_ref =
            session_ref_from_report("herdr:droid", "droid", Some("droid-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "droid-id");
        assert!(session_ref_from_report(
            "herdr:droid",
            "droid",
            None,
            Some("/tmp/droid-session".into())
        )
        .is_none());

        let session_ref =
            session_ref_from_report("herdr:kimi", "kimi", Some("kimi-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "kimi-id");

        let session_ref =
            session_ref_from_report("herdr:kilo", "kilo", Some("kilo-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "kilo-id");

        let session_ref =
            session_ref_from_report("herdr:qodercli", "qodercli", Some("qoder-id".into()), None)
                .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "qoder-id");
    }

    #[test]
    fn ids_are_data_not_shell_text() {
        let id = "abc; rm -rf /";
        let codex_plan = plan("herdr:codex", "codex", &AgentSessionRef::id(id).unwrap()).unwrap();
        assert_eq!(codex_plan.argv, vec!["codex", "resume", id]);

        let copilot_plan = plan(
            "herdr:copilot",
            "copilot",
            &AgentSessionRef::id(id).unwrap(),
        )
        .unwrap();
        assert_eq!(copilot_plan.argv, vec!["copilot", "--resume=abc; rm -rf /"]);
    }

    #[test]
    fn launch_argv_is_resumed_verbatim_with_resume_suffix() {
        let launch = vec![
            "claude".to_string(),
            "--dangerously-skip-permissions".to_string(),
            "--model".to_string(),
            "opus".to_string(),
        ];
        let plan = plan_with_launch_argv(
            "herdr:claude",
            "claude",
            &AgentSessionRef::id("claude-session").unwrap(),
            Some(&launch),
        )
        .unwrap();
        assert_eq!(
            plan.argv,
            vec![
                "claude",
                "--dangerously-skip-permissions",
                "--model",
                "opus",
                "--resume",
                "claude-session",
            ]
        );
    }

    #[test]
    fn missing_or_empty_launch_argv_falls_back_to_canonical_program() {
        let session = AgentSessionRef::id("claude-session").unwrap();
        let expected = vec!["claude", "--resume", "claude-session"];

        let with_none = plan_with_launch_argv("herdr:claude", "claude", &session, None).unwrap();
        let empty: Vec<String> = Vec::new();
        let with_empty =
            plan_with_launch_argv("herdr:claude", "claude", &session, Some(&empty)).unwrap();

        assert_eq!(with_none.argv, expected);
        assert_eq!(with_empty.argv, expected);
        // The historical 3-arg entry point keeps the same fallback behavior.
        assert_eq!(
            with_none.argv,
            plan("herdr:claude", "claude", &session).unwrap().argv
        );
    }

    #[test]
    fn launch_argv_preserves_original_program_token() {
        let launch = vec!["/usr/bin/claude".to_string()];
        let plan = plan_with_launch_argv(
            "herdr:claude",
            "claude",
            &AgentSessionRef::id("claude-session").unwrap(),
            Some(&launch),
        )
        .unwrap();
        assert_eq!(
            plan.argv,
            vec!["/usr/bin/claude", "--resume", "claude-session"]
        );
    }

    #[test]
    fn launch_argv_appends_codex_resume_subcommand_after_flags() {
        let launch = vec![
            "codex".to_string(),
            "--model".to_string(),
            "gpt-5".to_string(),
        ];
        let plan = plan_with_launch_argv(
            "herdr:codex",
            "codex",
            &AgentSessionRef::id("codex-session").unwrap(),
            Some(&launch),
        )
        .unwrap();
        assert_eq!(
            plan.argv,
            vec!["codex", "--model", "gpt-5", "resume", "codex-session"]
        );
    }

    #[test]
    fn launch_argv_strips_existing_claude_resume_selector() {
        let launch = vec![
            "claude".to_string(),
            "--dangerously-skip-permissions".to_string(),
            "--resume".to_string(),
            "old-session".to_string(),
        ];
        let plan = plan_with_launch_argv(
            "herdr:claude",
            "claude",
            &AgentSessionRef::id("new-session").unwrap(),
            Some(&launch),
        )
        .unwrap();
        assert_eq!(
            plan.argv,
            vec![
                "claude",
                "--dangerously-skip-permissions",
                "--resume",
                "new-session",
            ]
        );
        // Exactly one resume selector survives in the replayed command.
        assert_eq!(plan.argv.iter().filter(|a| *a == "--resume").count(), 1);
    }

    #[test]
    fn launch_argv_strips_claude_continue_and_short_flags() {
        let launch = vec![
            "claude".to_string(),
            "-c".to_string(),
            "-r".to_string(),
            "old-session".to_string(),
            "--model".to_string(),
            "opus".to_string(),
        ];
        let plan = plan_with_launch_argv(
            "herdr:claude",
            "claude",
            &AgentSessionRef::id("new-session").unwrap(),
            Some(&launch),
        )
        .unwrap();
        assert_eq!(
            plan.argv,
            vec!["claude", "--model", "opus", "--resume", "new-session"]
        );
    }

    #[test]
    fn launch_argv_strips_bare_resume_and_eq_form() {
        // A bare `--resume` (interactive picker) must not swallow the next flag.
        let bare = vec![
            "claude".to_string(),
            "--resume".to_string(),
            "--model".to_string(),
            "opus".to_string(),
        ];
        assert_eq!(
            plan_with_launch_argv(
                "herdr:claude",
                "claude",
                &AgentSessionRef::id("new-session").unwrap(),
                Some(&bare),
            )
            .unwrap()
            .argv,
            vec!["claude", "--model", "opus", "--resume", "new-session"]
        );

        let eq_form = vec!["claude".to_string(), "--resume=old".to_string()];
        assert_eq!(
            plan_with_launch_argv(
                "herdr:claude",
                "claude",
                &AgentSessionRef::id("new-session").unwrap(),
                Some(&eq_form),
            )
            .unwrap()
            .argv,
            vec!["claude", "--resume", "new-session"]
        );
    }

    #[test]
    fn launch_argv_strips_codex_resume_subcommand_but_keeps_config() {
        let launch = vec![
            "codex".to_string(),
            "-c".to_string(),
            "model=gpt-5".to_string(),
            "resume".to_string(),
            "old-session".to_string(),
        ];
        let plan = plan_with_launch_argv(
            "herdr:codex",
            "codex",
            &AgentSessionRef::id("new-session").unwrap(),
            Some(&launch),
        )
        .unwrap();
        // codex `-c` is a config override, not `--continue`; it must survive,
        // while the existing `resume <id>` subcommand is replaced.
        assert_eq!(
            plan.argv,
            vec!["codex", "-c", "model=gpt-5", "resume", "new-session"]
        );
        assert_eq!(plan.argv.iter().filter(|a| *a == "resume").count(), 1);
    }

    #[test]
    fn launch_argv_strips_session_selector_for_session_agents() {
        let launch = vec![
            "kimi".to_string(),
            "--session".to_string(),
            "old".to_string(),
        ];
        assert_eq!(
            plan_with_launch_argv(
                "herdr:kimi",
                "kimi",
                &AgentSessionRef::id("new").unwrap(),
                Some(&launch),
            )
            .unwrap()
            .argv,
            vec!["kimi", "--session", "new"]
        );
    }

    #[test]
    fn launch_argv_without_selectors_is_replayed_verbatim() {
        let launch = vec![
            "claude".to_string(),
            "--dangerously-skip-permissions".to_string(),
            "--model".to_string(),
            "opus".to_string(),
        ];
        assert_eq!(
            plan_with_launch_argv(
                "herdr:claude",
                "claude",
                &AgentSessionRef::id("s").unwrap(),
                Some(&launch),
            )
            .unwrap()
            .argv,
            vec![
                "claude",
                "--dangerously-skip-permissions",
                "--model",
                "opus",
                "--resume",
                "s",
            ]
        );
    }

    #[test]
    fn launch_argv_is_ignored_for_unsupported_sources() {
        let launch = vec!["claude".to_string(), "--foo".to_string()];
        assert!(plan_with_launch_argv(
            "custom:claude",
            "claude",
            &AgentSessionRef::id("session").unwrap(),
            Some(&launch),
        )
        .is_none());
    }

    #[test]
    fn planner_rejects_path_refs_for_id_only_agents() {
        let hermes_session = absolute_test_path("hermes-session");
        let opencode_session = absolute_test_path("opencode-session");
        let kilo_session = absolute_test_path("kilo-session");
        let copilot_session = absolute_test_path("copilot-session");
        assert!(plan(
            "herdr:hermes",
            "hermes",
            &AgentSessionRef::path(&hermes_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:opencode",
            "opencode",
            &AgentSessionRef::path(&opencode_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:kilo",
            "kilo",
            &AgentSessionRef::path(&kilo_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:copilot",
            "copilot",
            &AgentSessionRef::path(&copilot_session).unwrap()
        )
        .is_none());
        assert!(session_ref_from_snapshot(
            "herdr:hermes",
            "hermes",
            AgentSessionRefKind::Id,
            "hermes-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:opencode",
            "opencode",
            AgentSessionRefKind::Id,
            "opencode-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:kilo",
            "kilo",
            AgentSessionRefKind::Id,
            "kilo-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:copilot",
            "copilot",
            AgentSessionRefKind::Id,
            "copilot-session"
        )
        .is_some());
    }
}
