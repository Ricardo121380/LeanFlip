mod args;
mod presentation;

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use args::{AccountCommand, Cli, Command, Import, Language, ProviderCommand};
use ccsl_core::ccswitch_export::{Preview, MAX_EXPORT_BYTES};
use ccsl_core::{AppError, Env, Provider, Tool};
use ccsl_service::oauth_http::{Endpoints, HttpTransport};
use ccsl_service::service::{ApiError, OpResult, PollResult};
use ccsl_service::Service;
use clap::{CommandFactory, FromArgMatches};
use presentation::{clean, table};
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
            "codex.oauth.live_sync_failed" => "Account credentials were refreshed and saved, but the Codex client could not be synchronized. Check the client path and permissions; the new credentials were retained.",
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

    fn done(lang: Language, zh: &str, en: &str) -> Self {
        Self::new(json!({}), lang.text(zh, en))
    }

    fn operation(result: OpResult, lang: Language, zh: &str, en: &str) -> Self {
        let count = result.warnings.len();
        let mut text = lang.text(zh, en).to_string();
        if !result.changed.is_empty() {
            text.push_str(&format!(
                "\n{} {}",
                lang.text("已更新客户端文件：", "Client files updated:"),
                result.changed.len()
            ));
        }
        for path in &result.changed {
            text.push_str(&format!("\n  {}", clean(&path.display().to_string())));
        }
        if count > 0 {
            text.push_str(&format!(
                "\n{} {count}\n{}",
                lang.text("配置警告：", "Configuration warnings:"),
                lang.text(
                    "使用前请运行 ccsl paths <tool>，检查相关配置。",
                    "Run ccsl paths <tool> and inspect the affected configuration before using it."
                )
            ));
        }
        Self::new(
            json!({"changed": result.changed, "warningCount": count}),
            text,
        )
    }
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

fn preview_output(preview: Preview, tools: &[Tool], lang: Language) -> Output {
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
    let importable = items
        .iter()
        .filter(|item| !item["targetId"].is_null())
        .count();
    let mut text = format!(
        "{}\n{} {importable} · {} {}",
        lang.text("导入预览 · 尚未保存", "Import preview · Nothing saved"),
        lang.text("可导入", "Ready"),
        lang.text("跳过", "Skipped"),
        items.len() - importable
    );
    if items.is_empty() {
        text.push_str(lang.text(
            "\n没有符合所选工具的卡片。请检查 --tool 筛选条件。",
            "\nNo cards match the selected tools. Check the --tool filter.",
        ));
    } else {
        let rows = items
            .iter()
            .map(|item| {
                let status = if !item["targetId"].is_null() {
                    lang.text("可导入", "Ready")
                } else if item["status"] == "duplicate" {
                    lang.text("重复", "Duplicate")
                } else {
                    lang.text("跳过", "Skipped")
                };
                let mut reason = item["reasonCode"]
                    .as_str()
                    .map(|code| presentation::reason(code, lang).to_string())
                    .unwrap_or_else(|| "—".into());
                if item["reasonCode"] == "renamed" {
                    reason.push_str(&format!(": {}", field(item, "targetId")));
                }
                for code in item["noteCodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    reason.push_str(&format!("; {}", presentation::reason(code, lang)));
                }
                vec![
                    field(item, "app"),
                    field(item, "sourceId"),
                    status.into(),
                    reason,
                ]
            })
            .collect::<Vec<_>>();
        text.push_str(&format!(
            "\n\n{}",
            table(
                &[
                    lang.text("工具", "TOOL"),
                    "ID",
                    lang.text("状态", "STATUS"),
                    lang.text("说明", "DETAIL")
                ],
                &rows
            )
        ));
    }
    text.push_str(lang.text("\n\n确认后可在原命令追加 --apply 保存；保留原有 --tool、--duplicate 和 --home 参数。", "\n\nTo save, add --apply to this command and keep its --tool, --duplicate, and --home options."));
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
    lang: Language,
) -> Result<Output, CliError> {
    let flow = service.codex_start_login(account)?;
    // The device_code stays in this process. Only the short, user-facing code is displayed.
    let prompt = if as_json {
        json!({"event": "device_login", "verificationUri": flow.verification_uri, "userCode": flow.user_code, "expiresIn": flow.expires_in}).to_string()
    } else {
        format!(
            "{}\n  {}\n{} {}\n{}",
            lang.text("请在浏览器中打开：", "Open this URL in your browser:"),
            clean(&flow.verification_uri),
            lang.text("输入验证码：", "Enter code:"),
            clean(&flow.user_code),
            lang.text(
                "等待确认中。登录成功后会自动继续；Ctrl+C 可退出。",
                "Waiting for confirmation. This command continues after sign-in; Ctrl+C exits."
            )
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
                    let text = format!(
                        "{} {}",
                        lang.text("已登录账号：", "Signed in:"),
                        clean(&account.id)
                    );
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
    let lang = cli.lang;
    let custom_home = cli.home.is_some();
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
    let result: Result<Output, CliError> = match cli.command {
        Command::Status => {
            let tools = service.overview()?;
            let rows = tools
                .iter()
                .map(|tool| {
                    let state = if tool.tool.is_coexist() {
                        if tool.live_readable {
                            format!("{} {}", lang.text("已启用", "Enabled"), tool.in_live.len())
                        } else {
                            lang.text("配置无法读取", "Unreadable config").into()
                        }
                    } else {
                        tool.current
                            .clone()
                            .unwrap_or_else(|| lang.text("尚未选择", "Not selected").into())
                    };
                    vec![
                        tool.tool.to_string(),
                        lang.text(
                            if tool.tool.is_coexist() {
                                "并存"
                            } else {
                                "切换"
                            },
                            if tool.tool.is_coexist() {
                                "Coexist"
                            } else {
                                "Switch"
                            },
                        )
                        .into(),
                        tool.card_count.to_string(),
                        state,
                        lang.text(
                            if tool.detected {
                                "已检测"
                            } else {
                                "未检测"
                            },
                            if tool.detected { "Found" } else { "Not found" },
                        )
                        .into(),
                    ]
                })
                .collect::<Vec<_>>();
            let mut text = format!("{}\n\n{}\n\n{}", lang.text("CC Switch Lite · 工具概览", "CC Switch Lite · Tool overview"),
                table(&[lang.text("工具", "TOOL"), lang.text("模式", "MODE"), lang.text("卡片", "CARDS"), lang.text("当前选择 / 启用数", "SELECTION / ENABLED"), lang.text("客户端配置", "CLIENT CONFIG")], &rows),
                lang.text("切换式状态来自已保存的选择；可用 ccsl list <tool> 查看卡片。", "Switch selections reflect the saved choice. Run ccsl list <tool> to inspect cards."));
            if tools.iter().all(|tool| tool.card_count == 0) {
                text.push_str(lang.text(
                    "\n还没有供应商。可用 ccsl import live <tool> 导入现有配置。",
                    "\nNo providers yet. Use ccsl import live <tool> to import existing settings.",
                ));
            }
            let tools = tools
                .into_iter()
                .map(|tool| {
                    json!({
                        "tool": tool.tool, "kind": tool.kind, "paths": tool.paths,
                        "detected": tool.detected, "cardCount": tool.card_count,
                        "current": tool.current, "currentName": tool.current_name,
                        "inLive": (tool.tool.is_coexist() && tool.live_readable).then_some(tool.in_live),
                        "liveReadable": tool.tool.is_coexist().then_some(tool.live_readable)
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
                        "enabled": (list.tool.is_coexist() && list.live_error.is_none()).then(|| list.in_live.contains(&p.id))
                    })
                })
                .collect::<Vec<_>>();
            let mut text = format!(
                "{} · {} {}",
                list.tool,
                providers.len(),
                lang.text("个供应商", "provider(s)")
            );
            if providers.is_empty() {
                text.push_str(&format!(
                    "\n\n{}\n  ccsl import live {}\n  ccsl provider add {} --file provider.json",
                    lang.text(
                        "暂无供应商。可导入客户端配置，或从 JSON 文件新增：",
                        "No providers yet. Import client settings or add a JSON card:"
                    ),
                    list.tool,
                    list.tool
                ));
            } else {
                let rows = providers
                    .iter()
                    .map(|p| {
                        let state = if list.tool.is_coexist() {
                            match p["enabled"].as_bool() {
                                Some(true) => lang.text("已启用", "Enabled"),
                                Some(false) => lang.text("未启用", "Disabled"),
                                None => lang.text("无法读取", "Unknown"),
                            }
                        } else if p["current"] == true {
                            lang.text("当前使用", "Current")
                        } else {
                            lang.text("未选中", "Not selected")
                        };
                        vec![
                            field(p, "id"),
                            field(p, "name"),
                            lang.text(
                                if p["official"] == true {
                                    "官方"
                                } else {
                                    "自定义"
                                },
                                if p["official"] == true {
                                    "Official"
                                } else {
                                    "Custom"
                                },
                            )
                            .into(),
                            state.into(),
                        ]
                    })
                    .collect::<Vec<_>>();
                text.push_str(&format!(
                    "\n\n{}\n\n{}\n  ccsl {} {} <provider-id>",
                    table(
                        &[
                            "ID",
                            lang.text("名称", "NAME"),
                            lang.text("类型", "TYPE"),
                            lang.text("状态", "STATE")
                        ],
                        &rows
                    ),
                    lang.text("下一步：", "Next:"),
                    if list.tool.is_coexist() {
                        "enable"
                    } else {
                        "use"
                    },
                    list.tool
                ));
            }
            if list.live_error.is_some() {
                text.push_str(&format!("\n\n{}\n  ccsl paths {}", lang.text("无法读取客户端配置，启用状态未知。请先检查路径和文件格式：", "Client configuration is unreadable; enabled state is unknown. Check paths and file format:"), list.tool));
            }
            Ok(Output::new(
                json!({"tool": list.tool, "kind": list.kind, "providers": providers,
                "liveReadable": list.tool.is_coexist().then_some(list.live_error.is_none()), "paths": list.paths}),
                text,
            ))
        }
        Command::Use { tool, provider_id } => Ok(Output::operation(
            service.switch_provider(tool.into(), &provider_id)?,
            lang,
            "供应商已切换。",
            "Provider switched.",
        )),
        Command::Enable { tool, provider_id } => Ok(Output::operation(
            service.set_in_live(tool.into(), &provider_id, true)?,
            lang,
            "供应商已启用。",
            "Provider enabled.",
        )),
        Command::Disable { tool, provider_id } => Ok(Output::operation(
            service.set_in_live(tool.into(), &provider_id, false)?,
            lang,
            "供应商已停用，卡片已保留。",
            "Provider disabled; its saved card is retained.",
        )),
        Command::Paths { tool } => {
            let env = service.env();
            let tools = tool
                .map(|t| vec![Tool::from(t)])
                .unwrap_or_else(|| Tool::ALL.to_vec());
            let paths = tools
                .iter()
                .map(|&tool| (tool.as_str(), env.live_paths(tool)))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mut text = format!(
                "{}\n  {}",
                lang.text("CC Switch Lite 数据目录", "CC Switch Lite data directory"),
                clean(&env.data_dir.display().to_string())
            );
            for (tool, files) in &paths {
                text.push_str(&format!("\n\n{tool}"));
                for path in files {
                    text.push_str(&format!(
                        "\n  {}  {}",
                        if path.exists() {
                            lang.text("[存在]", "[exists]")
                        } else {
                            lang.text("[未创建]", "[missing]")
                        },
                        clean(&path.display().to_string())
                    ));
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
            let text = if result.created.is_empty() {
                lang.text("没有新增卡片。当前配置可能已导入，或没有可导入的供应商。", "No cards added. The current configuration may already be imported or contain no providers.").to_string()
            } else {
                format!(
                    "{} {} {}",
                    lang.text("已导入", "Imported"),
                    result.created.len(),
                    lang.text("张供应商卡片。", "provider card(s).")
                )
            };
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
                return Ok(preview_output(preview, &tools, lang));
            }
            let selected = if tools.is_empty() {
                Tool::ALL.to_vec()
            } else {
                tools
            };
            let report = service.export_apply(selected, duplicate.into())?;
            let text = format!(
                "{} {} · {} {}\n{}",
                lang.text("已保存卡片", "Cards saved:"),
                report.created.len(),
                lang.text("已跳过", "Skipped:"),
                report.skipped,
                lang.text(
                    "可用 ccsl list <tool> 查看，再通过 use 或 enable 应用到客户端。",
                    "Run ccsl list <tool> to inspect cards, then use use or enable to apply them."
                )
            );
            Ok(Output::new(json!(report), text))
        }
        Command::Provider { action } => match action {
            ProviderCommand::Add {
                tool,
                input,
                activate,
            } => Ok(Output::operation(
                service.add_provider(tool.into(), provider(&input.file)?, activate)?,
                lang,
                if activate {
                    "供应商已保存并应用。"
                } else {
                    "供应商已保存，可用 use 或 enable 应用。"
                },
                if activate {
                    "Provider saved and applied."
                } else {
                    "Provider saved. Apply it with use or enable."
                },
            )),
            ProviderCommand::Update { tool, input } => Ok(Output::operation(
                service.update_provider(tool.into(), provider(&input.file)?)?,
                lang,
                "供应商已更新。",
                "Provider updated.",
            )),
            ProviderCommand::Delete { tool, provider_id } => {
                service.delete_provider(tool.into(), &provider_id)?;
                Ok(Output::done(
                    lang,
                    "供应商卡片已删除。",
                    "Provider card deleted.",
                ))
            }
        },
        Command::Accounts { action } => match action {
            AccountCommand::List => {
                let accounts = service.codex_accounts_checked()?;
                let text = if accounts.is_empty() {
                    lang.text("Codex · 暂无托管账号\n\n运行以下命令，通过浏览器完成登录：\n  ccsl accounts login", "Codex · No managed accounts\n\nSign in through your browser:\n  ccsl accounts login").to_string()
                } else {
                    let rows = accounts
                        .iter()
                        .map(|account| {
                            vec![
                                account.id.clone(),
                                account.email.clone().unwrap_or_else(|| "—".into()),
                                if account.is_default {
                                    lang.text("默认", "Default").into()
                                } else {
                                    "—".into()
                                },
                                lang.text(
                                    if account.reauth_required {
                                        "需要重新登录"
                                    } else {
                                        "已登录"
                                    },
                                    if account.reauth_required {
                                        "Sign-in required"
                                    } else {
                                        "Signed in"
                                    },
                                )
                                .into(),
                            ]
                        })
                        .collect::<Vec<_>>();
                    format!(
                        "Codex · {} {}\n\n{}\n\n{}",
                        accounts.len(),
                        lang.text("个托管账号", "managed account(s)"),
                        table(
                            &[
                                "ID",
                                lang.text("邮箱", "EMAIL"),
                                lang.text("默认账号", "DEFAULT"),
                                lang.text("登录状态", "SIGN-IN")
                            ],
                            &rows
                        ),
                        lang.text(
                            "需重新登录时：ccsl accounts login --account <account-id>",
                            "To reauthenticate: ccsl accounts login --account <account-id>"
                        )
                    )
                };
                Ok(Output::new(json!({"accounts": accounts}), text))
            }
            AccountCommand::Login { account, timeout } => {
                login(&service, account.as_deref(), timeout, cli.json, lang)
            }
            AccountCommand::Default { account_id } => {
                service.codex_set_default(&account_id)?;
                Ok(Output::done(
                    lang,
                    "默认账号已更新。",
                    "Default account updated.",
                ))
            }
            AccountCommand::Remove { account_id } => {
                service.codex_remove_account(&account_id)?;
                Ok(Output::done(
                    lang,
                    "本地托管账号已移除。",
                    "Local managed account removed.",
                ))
            }
            AccountCommand::Refresh { account_id } => {
                service.codex_refresh_account(&account_id)?;
                Ok(Output::done(
                    lang,
                    "账号凭据已刷新并保存。",
                    "Account credentials refreshed and saved.",
                ))
            }
        },
        Command::Recover { tool } => {
            let notices = match tool {
                Some(tool) => service.recover_tool(tool.into())?,
                None => service.recover_all()?,
            };
            let text = if notices.is_empty() {
                lang.text("没有待恢复的中断写入。", "No interrupted writes found.")
                    .into()
            } else {
                format!("{} {}\n{}", lang.text("恢复处理已完成，提示数：", "Recovery completed; notices:"), notices.len(),
                    lang.text("请用 ccsl paths <tool> 定位并检查相关配置后再使用。", "Run ccsl paths <tool> and inspect the affected configuration before using it."))
            };
            Ok(Output::new(json!({"noticeCount": notices.len()}), text))
        }
    };
    let mut output = result?;
    let recovery_count = service.recovery().len();
    if recovery_count > 0 {
        output.data["recoveryNoticeCount"] = json!(recovery_count);
        output.text.push_str(&format!(
            "\n{} {recovery_count}\n{}",
            lang.text("中断写入恢复提示：", "Interrupted-write recovery notices:"),
            lang.text(
                "请检查相关客户端配置。",
                "Inspect the affected client configuration."
            )
        ));
    }
    if custom_home {
        output.text.push_str(lang.text(
            "\n\n当前使用自定义 HOME；后续命令请保留相同的 --home 参数。",
            "\n\nUsing a custom HOME; keep the same --home option in subsequent commands.",
        ));
    }
    Ok(output)
}

fn error_output(error: CliError, as_json: bool, lang: Language) -> ExitCode {
    let line = if as_json {
        json!({"ok": false, "error": {"code": error.code, "message": error.message}}).to_string()
    } else {
        presentation::error(&error.code, error.message, lang)
    };
    let _ = writeln!(io::stderr().lock(), "{line}");
    ExitCode::from(error.exit)
}

fn write_stdout(mut writer: impl Write, line: &str, as_json: bool) -> ExitCode {
    match writeln!(writer, "{line}") {
        Ok(()) => ExitCode::SUCCESS,
        // `ccsl list ... | head` is a normal way to read terminal output. A
        // closed consumer is not an operation failure. Keep JSON's exit contract.
        Err(error) if !as_json && error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(1),
    }
}

fn main() -> ExitCode {
    let raw = std::env::args_os().collect::<Vec<_>>();
    let as_json = raw.iter().any(|arg| arg == "--json");
    let lang = presentation::language_before_parse(&raw);
    let command = presentation::help(Cli::command(), lang);
    let parsed = command
        .clone()
        .try_get_matches_from(&raw)
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            let invalid = CliError::input(
                "invalid_arguments",
                "Invalid arguments. Run ccsl --help or ccsl <command> --help for usage.",
            );
            if as_json {
                return error_output(invalid, true, lang);
            }
            // Let clap identify the valid command prefix. Never display its
            // original diagnostic, which may echo arbitrary input or credentials.
            let partial = Cli::command()
                .ignore_errors(true)
                .try_get_matches_from(&raw)
                .ok();
            let mut help = presentation::matched_help(&command, partial.as_ref());
            if matches!(
                error.kind(),
                clap::error::ErrorKind::MissingSubcommand
                    | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                let title = if help.get_name() == "ccsl" {
                    lang.text("CC Switch Lite · 快速入门", "CC Switch Lite · Quick start")
                } else {
                    lang.text("命令帮助", "Command help")
                };
                return write_stdout(
                    io::stdout().lock(),
                    &format!("{title}\n\n{}", help.render_help()),
                    false,
                );
            }
            let exit = error_output(invalid, false, lang);
            let _ = writeln!(io::stderr().lock(), "\n{}", help.render_help());
            return exit;
        }
    };
    let as_json = cli.json;
    let lang = cli.lang;
    match run(cli) {
        Ok(output) => {
            let line = if as_json {
                json!({"ok": true, "data": output.data}).to_string()
            } else {
                output.text
            };
            write_stdout(io::stdout().lock(), &line, as_json)
        }
        Err(error) => error_output(error, as_json, lang),
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    struct FailedWriter(io::ErrorKind);

    impl Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn closed_human_output_is_success_but_json_and_other_io_errors_are_not() {
        assert_eq!(
            write_stdout(FailedWriter(io::ErrorKind::BrokenPipe), "output", false),
            ExitCode::SUCCESS
        );
        assert_eq!(
            write_stdout(FailedWriter(io::ErrorKind::BrokenPipe), "{}", true),
            ExitCode::from(1)
        );
        assert_eq!(
            write_stdout(
                FailedWriter(io::ErrorKind::PermissionDenied),
                "output",
                false
            ),
            ExitCode::from(1)
        );
    }
}
