//! Launch options of agent processes, kept across a server restart.
//!
//! herdr records the command line of an agent process when it detects the
//! agent ([`capture_launch_argv`]). At restore, [`with_launch_options`] puts the
//! options of that command line that do not select a session into the resume
//! command from [`super::plan`]. Only agents with a [`LaunchSpec`] get this;
//! the others resume with the plain upstream command.

use serde::{Deserialize, Serialize};

use super::AgentResumePlan;

/// The command line that started an agent, as herdr saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLaunch {
    /// Agent label, for example `claude`.
    pub agent: String,
    /// The command line. argv0 is the program name, not a path.
    pub argv: Vec<String>,
}

/// How many values an option takes, the way commander (claude) and clap
/// (codex) read their arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arity {
    Flag,
    Value,
    /// The next token, if it is not an option.
    OptionalValue,
    /// All next tokens that are not options.
    Variadic,
}

use Arity::{Flag, OptionalValue, Value, Variadic};

struct LaunchSpec {
    options: &'static [(&'static str, Arity)],
    /// Options that select a session or start a one-time task.
    dropped: &'static [&'static str],
    /// Subcommands that do not start an interactive session.
    other_subcommands: &'static [&'static str],
    /// Index in the resume command where the options go.
    insert_at: usize,
}

/// From `claude --help` (Claude Code 2.1.280).
const CLAUDE: LaunchSpec = LaunchSpec {
    options: &[
        ("--add-dir", Variadic),
        ("--agent", Value),
        ("--agents", Value),
        ("--allow-dangerously-skip-permissions", Flag),
        ("--allowedTools", Variadic),
        ("--allowed-tools", Variadic),
        ("--append-system-prompt", Value),
        ("--autocompact", Value),
        ("--ax-screen-reader", Flag),
        ("--bg", Flag),
        ("--background", Flag),
        ("--bare", Flag),
        ("--betas", Variadic),
        ("--brief", Flag),
        ("--chrome", Flag),
        ("--cloud", OptionalValue),
        ("-c", Flag),
        ("--continue", Flag),
        ("--dangerously-skip-permissions", Flag),
        ("-d", OptionalValue),
        ("--debug", OptionalValue),
        ("--debug-file", Value),
        ("--desktop", Flag),
        ("--disable-slash-commands", Flag),
        ("--disallowedTools", Variadic),
        ("--disallowed-tools", Variadic),
        ("--effort", Value),
        ("--environment", Value),
        ("--exclude-dynamic-system-prompt-sections", Flag),
        ("--fallback-model", Value),
        ("--file", Variadic),
        ("--fork-session", Flag),
        ("--forward-subagent-text", Flag),
        ("--from-pr", OptionalValue),
        ("-h", Flag),
        ("--help", Flag),
        ("--ide", Flag),
        ("--include-hook-events", Flag),
        ("--include-partial-messages", Flag),
        ("--input-format", Value),
        ("--json-schema", Value),
        ("--max-budget-usd", Value),
        ("--mcp-config", Variadic),
        ("--model", Value),
        ("-n", Value),
        ("--name", Value),
        ("--no-chrome", Flag),
        ("--no-session-persistence", Flag),
        ("--output-format", Value),
        ("--permission-mode", Value),
        ("--permission-prompts", Value),
        ("--permission-prompt-tool", Value),
        ("--plugin-dir", Value),
        ("--plugin-url", Value),
        ("-p", Flag),
        ("--print", Flag),
        ("--prompt-suggestions", OptionalValue),
        ("--remote-control", OptionalValue),
        ("--remote-control-session-name-prefix", Value),
        ("--replay-user-messages", Flag),
        ("--restricted", Flag),
        ("-r", OptionalValue),
        ("--resume", OptionalValue),
        ("--safe-mode", Flag),
        ("--session-id", Value),
        ("--setting-sources", Value),
        ("--settings", Value),
        ("--strict-mcp-config", Flag),
        ("--system-prompt", Value),
        ("--system-prompt-snapshot", Value),
        ("--teleport", OptionalValue),
        ("--tmux", Flag),
        ("--tools", Variadic),
        ("--verbose", Flag),
        ("-v", Flag),
        ("--version", Flag),
        ("-w", OptionalValue),
        ("--worktree", OptionalValue),
    ],
    dropped: &[
        "-r",
        "--resume",
        "-c",
        "--continue",
        "--session-id",
        "--fork-session",
        "--from-pr",
        "--teleport",
        "--cloud",
        "-p",
        "--print",
        "--bg",
        "--background",
        "-w",
        "--worktree",
        "--tmux",
        "-h",
        "--help",
        "-v",
        "--version",
    ],
    other_subcommands: &[
        "agents",
        "attach",
        "auth",
        "auto-mode",
        "doctor",
        "gateway",
        "import",
        "install",
        "kill",
        "logs",
        "mcp",
        "plugin",
        "plugins",
        "purge",
        "respawn",
        "rm",
        "setup-token",
        "stop",
        "ultrareview",
        "update",
        "upgrade",
    ],
    // claude <options> --resume <id>
    insert_at: 1,
};

/// From `codex --help` and `codex resume --help`.
const CODEX: LaunchSpec = LaunchSpec {
    options: &[
        ("-a", Value),
        ("--ask-for-approval", Value),
        ("--add-dir", Value),
        ("--all", Flag),
        ("--approve-for-me", Flag),
        ("-c", Value),
        ("--config", Value),
        ("-C", Value),
        ("--cd", Value),
        ("--dangerously-bypass-approvals-and-sandbox", Flag),
        ("--dangerously-bypass-hook-trust", Flag),
        ("--disable", Value),
        ("--enable", Value),
        ("-h", Flag),
        ("--help", Flag),
        ("-i", Variadic),
        ("--image", Variadic),
        ("--include-non-interactive", Flag),
        ("--last", Flag),
        ("--local-provider", Value),
        ("-m", Value),
        ("--model", Value),
        ("--no-alt-screen", Flag),
        ("--no-daemon", Flag),
        ("--oss", Flag),
        ("-p", Value),
        ("--profile", Value),
        ("--remote", Value),
        ("--remote-auth-token-env", Value),
        ("-s", Value),
        ("--sandbox", Value),
        ("--search", Flag),
        ("--strict-config", Flag),
        ("-V", Flag),
        ("--version", Flag),
        ("--worktree", Flag),
    ],
    dropped: &[
        "--last",
        "--all",
        "--include-non-interactive",
        "-i",
        "--image",
        "--worktree",
        "-h",
        "--help",
        "-V",
        "--version",
    ],
    other_subcommands: &[
        "agents",
        "app-server",
        "apply",
        "archive",
        "cloud",
        "completion",
        "debug",
        "delete",
        "doctor",
        "e",
        "exec",
        "exec-server",
        "features",
        "fork",
        "help",
        "login",
        "logout",
        "mcp",
        "migrate-nfs-db",
        "migrate-rollouts",
        "plugin",
        "queue",
        "remote-control",
        "review",
        "sandbox",
        "unarchive",
        "update",
    ],
    // codex resume <options> <id>
    insert_at: 2,
};

fn launch_spec(agent: &str) -> Option<&'static LaunchSpec> {
    match agent {
        "claude" => Some(&CLAUDE),
        "codex" => Some(&CODEX),
        _ => None,
    }
}

/// The command line to record for a detected agent process: `None` unless the
/// agent has a launch spec and argv0 resolves to that agent.
pub fn capture_launch_argv(agent: crate::detect::Agent, argv: &[String]) -> Option<Vec<String>> {
    launch_spec(crate::detect::agent_label(agent))?;
    direct_launch_argv(agent, argv)
}

/// A launch from a snapshot of the fork before v0.9.5, which kept the agent
/// command line in the pane's `launch_argv`. `None` when that command line is
/// not a direct invocation of `agent`.
pub fn legacy_agent_launch(agent: &str, launch_argv: &[String]) -> Option<AgentLaunch> {
    let argv = direct_launch_argv(crate::detect::parse_agent_label(agent)?, launch_argv)?;
    Some(AgentLaunch {
        agent: agent.to_owned(),
        argv,
    })
}

/// `argv` with argv0 reduced to its file name, when argv0 is `agent` itself.
/// `None` for wrapped invocations such as `node /path/cli.js`.
fn direct_launch_argv(agent: crate::detect::Agent, argv: &[String]) -> Option<Vec<String>> {
    let program = argv.first()?.trim_matches(|c| matches!(c, '"' | '\''));
    let program = std::path::Path::new(program).file_name()?.to_str()?;
    (crate::detect::parse_agent_label(program) == Some(agent)).then(|| {
        std::iter::once(program.to_owned())
            .chain(argv[1..].iter().cloned())
            .collect()
    })
}

/// Put the saved launch options into a resume plan. The plan stays as it is
/// when there is no launch for this agent, no option survives, or the result
/// is not a valid resume command.
pub fn with_launch_options(plan: AgentResumePlan, launch: Option<&AgentLaunch>) -> AgentResumePlan {
    match launch.and_then(|launch| argv_with_launch_options(&plan, launch)) {
        Some(argv) => AgentResumePlan { argv, ..plan },
        None => plan,
    }
}

fn argv_with_launch_options(plan: &AgentResumePlan, launch: &AgentLaunch) -> Option<Vec<String>> {
    if launch.agent != plan.agent {
        return None;
    }
    let spec = launch_spec(&plan.agent)?;
    let options = launch_options(spec, &launch.argv).filter(|options| !options.is_empty())?;
    let (head, tail) = plan.argv.split_at_checked(spec.insert_at)?;
    let argv = [head, &options, tail].concat();
    match super::validate_resume_argv(&argv) {
        Ok(()) => Some(argv),
        Err(reason) => {
            tracing::debug!(agent = %plan.agent, %reason, "resume without launch options");
            None
        }
    }
}

/// The options of `argv[1..]` that do not select a session. Positional
/// arguments (prompts, subcommands, session ids) are left out. `None` when
/// `argv` runs a subcommand other than an interactive session, or when an
/// unknown option makes the next token ambiguous.
fn launch_options(spec: &LaunchSpec, argv: &[String]) -> Option<Vec<String>> {
    let mut options = Vec::new();
    let mut first_positional = true;
    let mut tokens = argv.iter().skip(1).peekable();
    while let Some(token) = tokens.next() {
        if token == "--" {
            // Everything after `--` is positional.
            break;
        }
        if !is_option(token) {
            if first_positional && spec.other_subcommands.contains(&token.as_str()) {
                return None;
            }
            first_positional = false;
            continue;
        }
        let (name, attached) = match token.split_once('=') {
            Some((name, _)) => (name, true),
            None => (token.as_str(), false),
        };
        let arity = spec
            .options
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, arity)| *arity);
        let mut taken = vec![token.clone()];
        if !attached {
            match arity {
                Some(Flag) => {}
                Some(Value) => taken.extend(tokens.next().cloned()),
                Some(OptionalValue) => {
                    taken.extend(tokens.next_if(|next| !is_option(next)).cloned())
                }
                Some(Variadic) => {
                    while let Some(next) = tokens.next_if(|next| !is_option(next)) {
                        taken.push(next.clone());
                    }
                }
                None if tokens.peek().is_some_and(|next| !is_option(next)) => {
                    tracing::debug!(option = %token, "launch option with unknown arity");
                    return None;
                }
                None => {}
            }
        }
        if !spec.dropped.contains(&name) {
            options.extend(taken);
        }
    }
    Some(options)
}

fn is_option(token: &str) -> bool {
    token.len() > 1 && token.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_resume::{plan, AgentSessionRef};
    use crate::detect::Agent;

    /// Split on spaces: for command lines whose tokens have no spaces.
    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn launch(agent: &str, argv: Vec<String>) -> AgentLaunch {
        AgentLaunch {
            agent: agent.into(),
            argv,
        }
    }

    fn resumed_from(agent: &str, session: &str, argv: Vec<String>) -> Vec<String> {
        let session = AgentSessionRef::id(session).unwrap();
        let plan = plan(&format!("herdr:{agent}"), agent, &session).unwrap();
        with_launch_options(plan, Some(&launch(agent, argv))).argv
    }

    fn resumed(agent: &str, session: &str, line: &str) -> Vec<String> {
        resumed_from(agent, session, words(line))
    }

    #[test]
    fn claude_keeps_flags_and_drops_the_old_session() {
        assert_eq!(
            resumed(
                "claude",
                "e7e4",
                "claude --dangerously-skip-permissions --resume e149"
            ),
            words("claude --dangerously-skip-permissions --resume e7e4")
        );
    }

    #[test]
    fn codex_options_go_after_the_resume_subcommand() {
        assert_eq!(
            resumed(
                "codex",
                "id",
                "codex -m gpt-6-astra -s danger-full-access -a never"
            ),
            words("codex resume -m gpt-6-astra -s danger-full-access -a never id")
        );
    }

    #[test]
    fn the_initial_prompt_is_not_replayed() {
        let mut argv = words("claude --model opus");
        argv.insert(1, "fix bug".into());
        assert_eq!(
            resumed_from("claude", "id", argv),
            words("claude --model opus --resume id")
        );
        let mut argv = words("claude --model opus --");
        argv.push("-p looks like a flag".into());
        assert_eq!(
            resumed_from("claude", "id", argv),
            words("claude --model opus --resume id")
        );
    }

    #[test]
    fn a_second_restart_gives_the_same_command() {
        let first = resumed("codex", "a", "codex -s danger-full-access");
        assert_eq!(
            resumed_from("codex", "b", first),
            words("codex resume -s danger-full-access b")
        );
    }

    #[test]
    fn option_values_follow_their_arity() {
        // -r takes a value only when one follows; --add-dir takes all values.
        assert_eq!(
            resumed("claude", "id", "claude -r --add-dir a b --verbose"),
            words("claude --add-dir a b --verbose --resume id")
        );
        assert_eq!(
            resumed("claude", "id", "claude --debug=hooks -c"),
            words("claude --debug=hooks --resume id")
        );
        assert_eq!(
            resumed("codex", "id", "codex -i a.png b.png --search prompt"),
            words("codex resume --search id")
        );
    }

    #[test]
    fn an_ambiguous_unknown_option_drops_all_options() {
        assert_eq!(
            resumed("claude", "id", "claude --model opus --unknown value"),
            words("claude --resume id")
        );
        assert_eq!(
            resumed("claude", "id", "claude --unknown=value --new-flag"),
            words("claude --unknown=value --new-flag --resume id")
        );
    }

    #[test]
    fn other_subcommands_get_no_options() {
        assert_eq!(
            resumed("codex", "id", "codex -m x exec task"),
            words("codex resume id")
        );
        assert_eq!(
            resumed("claude", "id", "claude --verbose mcp list"),
            words("claude --resume id")
        );
    }

    #[test]
    fn plan_stays_when_the_launch_does_not_fit() {
        let session = AgentSessionRef::id("id").unwrap();
        let claude = plan("herdr:claude", "claude", &session).unwrap();
        let unchanged = |launch: AgentLaunch| with_launch_options(claude.clone(), Some(&launch));

        assert_eq!(unchanged(launch("codex", words("codex -m x"))), claude);
        assert_eq!(with_launch_options(claude.clone(), None), claude);
        let mut apostrophe = words("claude --append-system-prompt");
        apostrophe.push("don't".into());
        assert_eq!(unchanged(launch("claude", apostrophe)), claude);

        let kimi = plan("herdr:kimi", "kimi", &session).unwrap();
        let kimi_launch = launch("kimi", words("kimi --yolo"));
        assert_eq!(with_launch_options(kimi.clone(), Some(&kimi_launch)), kimi);
    }

    #[test]
    fn launch_options_keep_the_dedupe_key() {
        let session = AgentSessionRef::id("id").unwrap();
        let claude = plan("herdr:claude", "claude", &session).unwrap();
        let flags = launch("claude", words("claude --verbose"));
        let decorated = with_launch_options(claude.clone(), Some(&flags));
        assert_ne!(decorated.argv, claude.argv);
        assert_eq!(decorated.dedupe_key, claude.dedupe_key);
    }

    #[test]
    fn dropped_options_have_a_known_arity() {
        for spec in [&CLAUDE, &CODEX] {
            for name in spec.dropped {
                assert!(
                    spec.options.iter().any(|(known, _)| known == name),
                    "{name} is dropped but has no arity"
                );
            }
        }
    }

    #[test]
    fn capture_needs_a_spec_and_a_direct_invocation() {
        assert_eq!(
            capture_launch_argv(Agent::Claude, &words("/nix/store/x/bin/claude --verbose")),
            Some(words("claude --verbose"))
        );
        assert_eq!(
            capture_launch_argv(Agent::Codex, &words("node /opt/codex/bin/codex")),
            None
        );
        assert_eq!(
            capture_launch_argv(Agent::Claude, &words("/usr/bin/codex")),
            None
        );
        assert_eq!(
            capture_launch_argv(Agent::Kimi, &words("kimi --yolo")),
            None
        );
        assert_eq!(capture_launch_argv(Agent::Claude, &[]), None);
    }

    #[test]
    fn legacy_launch_must_belong_to_the_session_agent() {
        assert_eq!(
            legacy_agent_launch("claude", &words("claude --dangerously-skip-permissions")),
            Some(launch(
                "claude",
                words("claude --dangerously-skip-permissions")
            ))
        );
        assert_eq!(
            legacy_agent_launch(
                "claude",
                &words("codex --dangerously-bypass-approvals-and-sandbox")
            ),
            None
        );
        assert_eq!(legacy_agent_launch("unknown-agent", &words("claude")), None);
    }
}
