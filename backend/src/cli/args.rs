use super::output::Failure;
use crate::limits::*;
use std::io::Read;

pub const HELP: &str = "tmx — access a Teamarness Workspace\n\nUsage:\n  tmx workspaces\n  tmx --workspace <id-or-exact-name> <command> [options]\n\nCommands:\n  workspaces   List locally configured gateways (offline)\n  status       Check authenticated Workspace identity\n  contracts    Discover one page; follow nextCursor even after an empty page\n  schema <contract> --major <number>\n  guide <contract> --major <number>\n  call <contract> <port> --major <number> (--json <value> | --input <file|->)\n  subscribe <contract> <port> --major <number> (--json <value> | --input <file|->)\n    (--for <duration> | --until-cancelled) [--max-events <n>]\n\nOptions:\n  --workspace <value>  Required for every gateway command; no default\n  --cursor <value>     Next contracts page's opaque cursor\n  --json <value>       Exactly one request JSON value\n  --input <file|->     Explicit file or stdin; exclusive with --json\n  --for <duration>    Positive integer ms/s/m/h; starts at ready\n  --until-cancelled   No CLI lifetime limit; exactly one lifetime flag required\n  --max-events <n>    Positive u32 bound; counts values only, not heartbeats\n  --help              Offline help\n  --version           Package version and CLI interaction major\n\nOrdinary results/errors are JSON; subscriptions are NDJSON. Diagnostics go to stderr.\nNo automatic retries or reconnects. Heartbeats are not domain events.\nSubscriptions require stdout pipe/terminal; save via a pipe to tee with shell pipefail.\nProvider pagination and version tokens remain Provider-owned.\n";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Workspaces,
    Status,
    Contracts,
    Schema,
    Guide,
    Call,
    Subscribe,
}
pub struct Args {
    pub command: Command,
    pub workspace: Option<String>,
    pub target: String,
    pub body: Vec<u8>,
    pub response_limit: usize,
    pub observation: Option<Observation>,
}
#[derive(Clone, Debug)]
pub struct Observation {
    pub contract: String,
    pub major: u16,
    pub port: String,
    pub duration: Option<std::time::Duration>,
    pub max_events: Option<u32>,
}
fn duration(text: &str) -> Option<std::time::Duration> {
    let (digits, scale) = [
        ("ms", 1_000_000u64),
        ("s", 1_000_000_000),
        ("m", 60_000_000_000),
        ("h", 3_600_000_000_000),
    ]
    .into_iter()
    .find_map(|(unit, scale)| text.strip_suffix(unit).map(|n| (n, scale)))?;
    let n = positive(digits)?.checked_mul(scale)?;
    let duration = std::time::Duration::from_nanos(n);
    std::time::Instant::now().checked_add(duration)?;
    Some(duration)
}
fn positive(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok().filter(|n| *n > 0)
}
fn invalid() -> Failure {
    Failure::usage("cli.invalid_arguments")
}
fn value(args: &[String], i: &mut usize) -> Result<String, Failure> {
    *i += 1;
    args.get(*i)
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or_else(invalid)
}
pub fn parse(args: &[String], stdin: &mut dyn Read) -> Result<Args, Failure> {
    let simple = |command| Args {
        command,
        workspace: None,
        target: String::new(),
        body: vec![],
        response_limit: 0,
        observation: None,
    };
    if args.is_empty() || args == ["--help"] {
        return Ok(simple(Command::Help));
    }
    if args == ["--version"] {
        return Ok(simple(Command::Version));
    }
    let (mut workspace, mut major, mut json, mut input, mut cursor) =
        (None, None, None, None, None);
    let (mut lifetime, mut max_events) = (None, None);
    let mut continuous = false;
    let mut positional = vec![];
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--until-cancelled" {
            if continuous {
                return Err(invalid());
            }
            continuous = true;
            i += 1;
            continue;
        }
        let slot = match args[i].as_str() {
            "--workspace" => Some(&mut workspace),
            "--major" => Some(&mut major),
            "--json" => Some(&mut json),
            "--input" => Some(&mut input),
            "--cursor" => Some(&mut cursor),
            "--for" => Some(&mut lifetime),
            "--max-events" => Some(&mut max_events),
            v if v.starts_with('-') => return Err(invalid()),
            _ => None,
        };
        if let Some(slot) = slot {
            if slot.is_some() {
                return Err(invalid());
            }
            *slot = Some(value(args, &mut i)?);
        } else {
            positional.push(args[i].as_str());
        }
        i += 1;
    }
    let cmd = match positional.first().copied() {
        Some("workspaces") => Command::Workspaces,
        Some("status") => Command::Status,
        Some("contracts") => Command::Contracts,
        Some("schema") => Command::Schema,
        Some("guide") => Command::Guide,
        Some("call") => Command::Call,
        Some("subscribe") => Command::Subscribe,
        _ => return Err(invalid()),
    };
    let invocation = matches!(cmd, Command::Call | Command::Subscribe);
    let selected = matches!(cmd, Command::Schema | Command::Guide) || invocation;
    if positional.len()
        != if invocation {
            3
        } else if selected {
            2
        } else {
            1
        }
        || (json.is_some() && input.is_some())
        || ((json.is_some() || input.is_some()) && !invocation)
        || (cursor.is_some() && cmd != Command::Contracts)
        || (major.is_some() && !selected)
        || (cmd == Command::Workspaces && workspace.is_some())
        || (cmd != Command::Subscribe && (continuous || lifetime.is_some() || max_events.is_some()))
        || (cmd == Command::Subscribe && continuous == lifetime.is_some())
    {
        return Err(invalid());
    }
    if cmd != Command::Workspaces && workspace.is_none() {
        return Err(Failure::usage("cli.workspace_required"));
    }
    if workspace
        .as_ref()
        .is_some_and(|w| w.len() > 1024 || w.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    let mut target = match cmd {
        Command::Workspaces => String::new(),
        Command::Status => "/v1".into(),
        Command::Contracts => "/v1/contracts".into(),
        Command::Call => "/v1/call".into(),
        Command::Subscribe => "/v1/subscribe".into(),
        _ => String::new(),
    };
    if selected {
        if !teamarness_package_contract::identity::is_valid_package_identity(positional[1]) {
            return Err(invalid());
        }
        let major = major
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|m| *m > 0)
            .ok_or_else(invalid)?;
        if !invocation {
            target = format!(
                "/v1/contracts/{}/{major}{}",
                positional[1],
                if cmd == Command::Guide { "/guide" } else { "" }
            );
        } else {
            let port = positional[2];
            if port.is_empty()
                || port.len() > 128
                || !port
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
            {
                return Err(invalid());
            }
            let bytes = if let Some(json) = json {
                json.into_bytes()
            } else if let Some(input) = input {
                let mut reader: Box<dyn Read + '_> = if input == "-" {
                    Box::new(stdin)
                } else {
                    Box::new(
                        std::fs::File::open(input)
                            .map_err(|_| Failure::usage("cli.input_failed"))?,
                    )
                };
                let mut bytes = vec![];
                reader
                    .by_ref()
                    .take(PORT_VALUE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Failure::usage("cli.input_failed"))?;
                bytes
            } else {
                return Err(Failure::usage("cli.input_required"));
            };
            if bytes.len() > PORT_VALUE_BYTES || !super::output::valid_json(&bytes) {
                return Err(Failure::usage("cli.invalid_json"));
            }
            let mut body = format!(
                "{{\"contract\":{},\"major\":{major},\"port\":{},\"request\":",
                serde_json::to_string(positional[1]).unwrap(),
                serde_json::to_string(port).unwrap()
            )
            .into_bytes();
            body.extend_from_slice(&bytes);
            body.push(b'}');
            let observation = if cmd == Command::Subscribe {
                Some(Observation {
                    contract: positional[1].into(),
                    major,
                    port: port.into(),
                    duration: lifetime
                        .as_deref()
                        .map(|s| duration(s).ok_or_else(invalid))
                        .transpose()?,
                    max_events: max_events
                        .as_deref()
                        .map(|s| {
                            positive(s)
                                .and_then(|n| u32::try_from(n).ok())
                                .ok_or_else(invalid)
                        })
                        .transpose()?,
                })
            } else {
                None
            };
            return Ok(Args {
                observation,
                command: cmd,
                workspace,
                target,
                body,
                response_limit: PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES,
            });
        }
    }
    if let Some(cursor) = cursor {
        if cursor.is_empty()
            || cursor.len() > MAX_HTTP_CURSOR_CHARS
            || !cursor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid());
        }
        target.push_str("?cursor=");
        target.push_str(&cursor);
    }
    if target.len() > MAX_TARGET_BYTES {
        return Err(invalid());
    }
    let response_limit = match cmd {
        Command::Schema => METADATA_BYTES,
        Command::Guide => PORT_VALUE_BYTES,
        _ => SUMMARY_BYTES,
    } + ENVELOPE_ALLOWANCE_BYTES;
    Ok(Args {
        command: cmd,
        workspace,
        target,
        body: vec![],
        response_limit,
        observation: None,
    })
}
