mod args;

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use args::{AccountCommand, Cli, Command, Import, ProviderCommand};
use ccsl_core::ccswitch_export::{Preview, MAX_EXPORT_BYTES};
use ccsl_core::{AppError, Env, Provider, Tool};
use ccsl_service::oauth_http::{Endpoints, HttpTransport};
use ccsl_service::service::{ApiError, OpResult, PollResult};
use ccsl_service::Service;
use clap::Parser;
use serde_json::{json, Value};

/// Core errors may include source text from a malformed configuration, including
/// credentials. The CLI deliberately emits codes and fixed descriptions only.
struct CliError {
    code: String,
    message: &'static str,
    exit: u8,
}

impl CliError {
    fn input(code: &str, message: &'static str) -> Self {
        Self {
            code: code.into(),
            message,
            exit: 2,
        }
    }

    fn io() -> Self {
        Self {
            code: "io".into(),
            message: "Cannot read or write the requested file. Check its path and permissions.",
            exit: 1,
        }
    }
}

impl From<ApiError> for CliError {
    fn from(error: ApiError) -> Self {
        let code = error.code();
        let message = match code {
            "invalid_input" => "Invalid provider or operation input. Check the command and provider JSON.",
            "not_found" => "The requested provider, account, or file was not found.",
            "conflict" => "Configuration changed concurrently. Inspect the current configuration and retry.",
            "config" | "json" => "Configuration cannot be read or validated. Inspect the files shown by ccsl paths.",
            "io" => "Cannot access a required file. Check paths and permissions.",
            "serialization" => "Configuration could not be serialized; the operation failed.",
            _ => "The operation failed. Use the error code to identify the cause; configuration contents are withheld.",
        };
        Self {
            code: code.to_owned(),
            message,
            exit: if code == "invalid_input" { 2 } else { 1 },
        }
    }
}

impl From<AppError> for CliError {
    fn from(error: AppError) -> Self {
        ApiError::from_core(error, true).into()
    }
}

struct Output {
    data: Value,
    text: String,
}

impl Output {
    fn new(data: Value, text: impl Into<String>) -> Self {
        Self {
            data,
            text: text.into(),
        }
    }

    fn done() -> Self {
        Self::new(json!({}), "Done.")
    }

    fn operation(result: OpResult) -> Self {
        let count = result.warnings.len();
        let mut text = format!("Done. Changed {} client file(s).", result.changed.len());
        for path in &result.changed {
            text.push_str(&format!("\n{}", clean(&path.display().to_string())));
        }
        if count > 0 {
            text.push_str(&format!(
                "\n{count} warning(s): inspect the affected configuration before using it."
            ));
        }
        Self::new(
            json!({"changed": result.changed, "warningCount": count}),
            text,
        )
    }
}

fn clean(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn read_limited(path: &Path, max: usize, allow_stdin: bool) -> Result<Vec<u8>, CliError> {
    let mut source: Box<dyn Read> = if allow_stdin && path == Path::new("-") {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(path).map_err(|_| CliError::io())?)
    };
    let mut bytes = Vec::new();
    source
        .by_ref()
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::io())?;
    if bytes.len() > max {
        return Err(CliError::input(
            "input_too_large",
            "The input exceeds the supported size limit.",
        ));
    }
    Ok(bytes)
}

fn provider(path: &Path) -> Result<Provider, CliError> {
    let bytes = read_limited(path, 1024 * 1024, true)?;
    serde_json::from_slice(&bytes).map_err(|_| CliError::input("invalid_provider_json", "Expected a provider JSON object with id, name, and settingsConfig. Input contents are withheld."))
}

fn preview_output(preview: Preview, tools: &[Tool]) -> Output {
    let items = preview
        .items
        .into_iter()
        .filter(|item| tools.is_empty() || item.tool.is_some_and(|tool| tools.contains(&tool)))
        .map(|item| {
            json!({
                "app": item.app,
                "tool": item.tool,
                "sourceId": item.source_id,
                "name": item.name,
                "status": item.status,
                "targetId": item.target_id,
                "reasonCode": item.reason.map(|reason| reason.code),
                "noteCodes": item.notes.into_iter().map(|reason| reason.code).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let mut text = String::from("Preview only; no cards saved.\nTOOL\tID\tSTATUS\tREASON");
    for item in &items {
        text.push_str(&format!(
            "\n{}\t{}\t{}\t{}",
            field(item, "app"),
            field(item, "sourceId"),
            field(item, "status"),
            field(item, "reasonCode")
        ));
    }
    Output::new(
        json!({"format": preview.format, "schemaVersion": preview.schema_version, "items": items}),
        text,
    )
}

fn field(value: &Value, name: &str) -> String {
    clean(value[name].as_str().unwrap_or("-"))
}

fn login(
    service: &Service,
    account: Option<&str>,
    timeout: u64,
    as_json: bool,
) -> Result<Output, CliError> {
    let flow = service.codex_start_login(account)?;
    // The device_code stays in this process. Only the short, user-facing code is displayed.
    let prompt = if as_json {
        json!({"event": "device_login", "verificationUri": flow.verification_uri, "userCode": flow.user_code, "expiresIn": flow.expires_in}).to_string()
    } else {
        format!(
            "Open {}\nEnter code: {}",
            clean(&flow.verification_uri),
            clean(&flow.user_code)
        )
    };
    let result = (|| {
        writeln!(io::stderr().lock(), "{prompt}").map_err(|_| CliError::io())?;
        let started = Instant::now();
        let limit = Duration::from_secs(timeout.min(flow.expires_in));
        let interval = Duration::from_secs(flow.interval.max(1));
        while started.elapsed() < limit {
            std::thread::sleep(interval.min(limit.saturating_sub(started.elapsed())));
            if started.elapsed() >= limit {
                break;
            }
            match service.codex_poll_login(&flow.device_code)? {
                PollResult::Pending => {}
                PollResult::Added { account } => {
                    let text = format!("Signed in: {}", clean(&account.id));
                    return Ok(Output::new(json!({"account": account}), text));
                }
            }
        }
        Err(CliError {
            code: "login_timeout".into(),
            message: "Device login timed out. Run accounts login again to obtain a new code.",
            exit: 1,
        })
    })();
    service.codex_cancel_login(&flow.device_code);
    result
}

fn run(cli: Cli) -> Result<Output, CliError> {
    match &cli.command {
        Command::Use { tool, .. } if Tool::from(*tool).is_coexist() => {
            return Err(CliError::input(
                "invalid_tool_mode",
                "Use enable or disable for a coexist-style tool.",
            ));
        }
        Command::Enable { tool, .. } | Command::Disable { tool, .. }
            if !Tool::from(*tool).is_coexist() =>
        {
            return Err(CliError::input(
                "invalid_tool_mode",
                "Use the use command for a switch-style tool.",
            ));
        }
        _ => {}
    }
    let base = cli
        .home
        .map(Env::with_home)
        .unwrap_or_else(Env::from_system);
    // Creating the transport does not send requests. Switching a managed Codex
    // provider can need a token refresh, so every mutating command can use it.
    let transport = HttpTransport::new(Endpoints::default()).map_err(|_| CliError {
        code: "transport_init".into(),
        message: "Cannot initialize the OAuth network transport.",
        exit: 1,
    })?;
    let service = Service::open(base, Some(Arc::new(transport)))?;
    match cli.command {
        Command::Status => {
            let tools = service.overview()?;
            let mut text = String::from("TOOL\tCARDS\tCURRENT (stored)\tENABLED\tDETECTED");
            for tool in &tools {
                text.push_str(&format!(
                    "\n{}\t{}\t{}\t{}\t{}",
                    tool.tool,
                    tool.card_count,
                    clean(tool.current.as_deref().unwrap_or("-")),
                    if tool.live_readable {
                        tool.in_live.len().to_string()
                    } else {
                        "unknown".into()
                    },
                    tool.detected
                ));
            }
            let tools = tools
                .into_iter()
                .map(|tool| {
                    json!({
                        "tool": tool.tool, "kind": tool.kind, "paths": tool.paths,
                        "detected": tool.detected, "cardCount": tool.card_count,
                        "current": tool.current, "currentName": tool.current_name,
                        "inLive": tool.live_readable.then_some(tool.in_live),
                        "liveReadable": tool.live_readable
                    })
                })
                .collect::<Vec<_>>();
            Ok(Output::new(json!({"tools": tools}), text))
        }
        Command::List { tool } => {
            let list = service.list_providers(tool.into())?;
            let providers = list
                .providers
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id, "name": p.name, "official": p.is_official_category(),
                        "current": list.current.as_deref() == Some(&p.id),
                        "enabled": list.live_error.is_none().then(|| list.in_live.contains(&p.id))
                    })
                })
                .collect::<Vec<_>>();
            let mut text = String::from("ID\tNAME\tCURRENT\tENABLED");
            for p in &providers {
                text.push_str(&format!(
                    "\n{}\t{}\t{}\t{}",
                    field(p, "id"),
                    field(p, "name"),
                    p["current"],
                    p["enabled"]
                ));
            }
            if list.live_error.is_some() {
                text.push_str("\nLive configuration is unreadable; enabled state is unknown.");
            }
            Ok(Output::new(
                json!({"tool": list.tool, "kind": list.kind, "providers": providers,
                "liveReadable": list.live_error.is_none(), "paths": list.paths}),
                text,
            ))
        }
        Command::Use { tool, provider_id } => Ok(Output::operation(
            service.switch_provider(tool.into(), &provider_id)?,
        )),
        Command::Enable { tool, provider_id } => Ok(Output::operation(service.set_in_live(
            tool.into(),
            &provider_id,
            true,
        )?)),
        Command::Disable { tool, provider_id } => Ok(Output::operation(service.set_in_live(
            tool.into(),
            &provider_id,
            false,
        )?)),
        Command::Paths { tool } => {
            let env = service.env();
            let tools = tool
                .map(|t| vec![Tool::from(t)])
                .unwrap_or_else(|| Tool::ALL.to_vec());
            let paths = tools
                .iter()
                .map(|&tool| (tool.as_str(), env.live_paths(tool)))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mut text = format!("Data: {}", clean(&env.data_dir.display().to_string()));
            for (tool, files) in &paths {
                for path in files {
                    text.push_str(&format!("\n{tool}: {}", clean(&path.display().to_string())));
                }
            }
            Ok(Output::new(
                json!({"home": env.home, "dataDir": env.data_dir, "tools": paths}),
                text,
            ))
        }
        Command::Import {
            source: Import::Live { tool },
        } => {
            let result = service.import_live(tool.into())?;
            let text = format!("Saved {} provider card(s).", result.created.len());
            Ok(Output::new(json!(result), text))
        }
        Command::Import {
            source:
                Import::File {
                    path,
                    apply,
                    duplicate,
                    tools,
                    ..
                },
        } => {
            let tools = tools.into_iter().map(Tool::from).collect::<Vec<_>>();
            let bytes = read_limited(&path, MAX_EXPORT_BYTES, false)?;
            let preview = service.export_load(&bytes, duplicate.into())?;
            if !apply {
                return Ok(preview_output(preview, &tools));
            }
            let selected = if tools.is_empty() {
                Tool::ALL.to_vec()
            } else {
                tools
            };
            let report = service.export_apply(selected, duplicate.into())?;
            let text = format!(
                "Saved {} provider card(s); skipped {}. Client files were not changed.",
                report.created.len(),
                report.skipped
            );
            Ok(Output::new(json!(report), text))
        }
        Command::Provider { action } => match action {
            ProviderCommand::Add {
                tool,
                input,
                activate,
            } => Ok(Output::operation(service.add_provider(
                tool.into(),
                provider(&input.file)?,
                activate,
            )?)),
            ProviderCommand::Update { tool, input } => Ok(Output::operation(
                service.update_provider(tool.into(), provider(&input.file)?)?,
            )),
            ProviderCommand::Delete { tool, provider_id } => {
                service.delete_provider(tool.into(), &provider_id)?;
                Ok(Output::done())
            }
        },
        Command::Accounts { action } => match action {
            AccountCommand::List => {
                let accounts = service.codex_accounts_checked()?;
                let mut text = String::from("ID\tEMAIL\tDEFAULT\tREAUTH REQUIRED");
                for account in &accounts {
                    text.push_str(&format!(
                        "\n{}\t{}\t{}\t{}",
                        clean(&account.id),
                        clean(account.email.as_deref().unwrap_or("-")),
                        account.is_default,
                        account.reauth_required
                    ));
                }
                Ok(Output::new(json!({"accounts": accounts}), text))
            }
            AccountCommand::Login { account, timeout } => {
                login(&service, account.as_deref(), timeout, cli.json)
            }
            AccountCommand::Default { account_id } => {
                service.codex_set_default(&account_id)?;
                Ok(Output::done())
            }
            AccountCommand::Remove { account_id } => {
                service.codex_remove_account(&account_id)?;
                Ok(Output::done())
            }
            AccountCommand::Refresh { account_id } => {
                service.codex_refresh_account(&account_id)?;
                Ok(Output::done())
            }
        },
        Command::Recover { tool } => {
            let notices = match tool {
                Some(tool) => service.recover_tool(tool.into())?,
                None => service.recover_all()?,
            };
            let text = if notices.is_empty() {
                "No interrupted writes found.".into()
            } else {
                format!("Recovery completed with {} notice(s). Inspect the affected client configuration before using it.", notices.len())
            };
            Ok(Output::new(json!({"noticeCount": notices.len()}), text))
        }
    }
}

fn error_output(error: CliError, as_json: bool) -> ExitCode {
    let line = if as_json {
        json!({"ok": false, "error": {"code": error.code, "message": error.message}}).to_string()
    } else {
        format!("Error [{}]: {}", error.code, error.message)
    };
    let _ = writeln!(io::stderr().lock(), "{line}");
    ExitCode::from(error.exit)
}

fn main() -> ExitCode {
    let as_json = std::env::args_os().any(|arg| arg == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            return error_output(
                CliError::input(
                    "invalid_arguments",
                    "Invalid arguments. Run ccsl --help or ccsl <command> --help for usage.",
                ),
                as_json,
            )
        }
    };
    match run(cli) {
        Ok(output) => {
            let line = if as_json {
                json!({"ok": true, "data": output.data}).to_string()
            } else {
                output.text
            };
            if writeln!(io::stdout().lock(), "{line}").is_err() {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(error) => error_output(error, as_json),
    }
}
