use crate::{
    args::Language,
    menu,
    presentation::{clean, table},
    read_limited, CliError, Output,
};
use ccsl_core::{ccswitch_import::Scan, Tool};
use ccsl_service::{
    service::{BackupPreview, RestoreOptions, RestoreReport, MAX_BACKUP_BYTES},
    Service,
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn password(
    file: Option<&Path>,
    confirm: bool,
    json: bool,
    lang: Language,
) -> Result<String, CliError> {
    if let Some(file) = file {
        let bytes = read_limited(file, 16 * 1024, false)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            CliError::input(
                "backup.password",
                "The password file must contain UTF-8 text.",
            )
        })?;
        return Ok(text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(&text)
            .to_owned());
    }
    if json || !menu::can_interact() {
        return Err(CliError::input("backup.password_required", "Use --password-file with a private file in scripts, or run this command in an interactive terminal."));
    }
    let mut prompt = dialoguer::Password::new()
        .with_prompt(lang.text("备份密码（不显示输入）", "Backup passphrase (hidden input)"));
    if confirm {
        prompt = prompt.with_confirmation(
            lang.text("再次输入密码", "Confirm passphrase"),
            lang.text("两次密码不一致", "Passphrases do not match"),
        );
    }
    prompt.interact().map_err(|_| menu::terminal_error())
}

pub(super) fn local_preview(scan: &Scan, tools: &[Tool], accounts: bool, lang: Language) -> Output {
    let items = scan.candidates.iter().filter(|candidate| tools.is_empty() || tools.contains(&candidate.tool)).map(|candidate| json!({
        "tool":candidate.tool,"id":candidate.provider.id,"name":candidate.provider.name,
        "canImport":candidate.skip_reason.is_none(),"reasonCode":candidate.skip_reason.as_ref().map(|reason| reason.code),
    })).collect::<Vec<_>>();
    let rows = items
        .iter()
        .map(|item| {
            vec![
                item["tool"].as_str().unwrap_or_default().into(),
                clean(item["name"].as_str().unwrap_or_default()),
                clean(item["id"].as_str().unwrap_or_default()),
                if item["canImport"] == true {
                    lang.text("可导入", "Ready").into()
                } else {
                    item["reasonCode"].as_str().unwrap_or("skipped").into()
                },
            ]
        })
        .collect::<Vec<_>>();
    let will_accounts = if accounts && (tools.is_empty() || tools.contains(&Tool::Codex)) {
        scan.accounts
    } else {
        0
    };
    let text = format!(
        "{}\n{}\n\n{}\n\n{} {} / {}\n{}",
        lang.text(
            "本机 CC Switch · 导入预览",
            "Local CC Switch · Import preview"
        ),
        clean(&scan.directory.display().to_string()),
        if rows.is_empty() {
            lang.text("没有发现供应商卡片。", "No provider cards found.")
                .into()
        } else {
            table(
                &[
                    lang.text("工具", "TOOL"),
                    lang.text("名称", "NAME"),
                    "ID",
                    lang.text("状态", "STATE"),
                ],
                &rows,
            )
        },
        lang.text(
            "托管账号：将导入 / 检测到",
            "Managed accounts: selected / found"
        ),
        will_accounts,
        scan.accounts,
        lang.text(
            "尚未写入。追加 --apply 确认导入；需要账号时加 --accounts。",
            "Nothing written. Add --apply to migrate; add --accounts to include managed logins."
        )
    );
    Output::new(
        json!({"directory":scan.directory,"source":scan.source,"items":items,"accounts":scan.accounts,"selectedAccounts":will_accounts}),
        text,
    )
}

pub(super) fn backup_preview(
    preview: &BackupPreview,
    options: &RestoreOptions,
    lang: Language,
) -> Output {
    let rows = preview
        .items
        .iter()
        .filter(|item| options.tools.contains(&item.tool))
        .map(|item| {
            vec![
                item.tool.to_string(),
                clean(&item.name),
                clean(&item.id),
                if item.duplicate {
                    lang.text("已存在", "Exists")
                } else {
                    lang.text("新增", "New")
                }
                .into(),
            ]
        })
        .collect::<Vec<_>>();
    let text = format!("{}\n\n{}\n\n{} {} · {} {} · {}\n{}",
        lang.text("LeanFlip · 已解密备份预览", "LeanFlip · Unlocked backup preview"),
        table(&[lang.text("工具", "TOOL"),lang.text("名称", "NAME"),"ID",lang.text("状态", "STATE")], &rows),
        lang.text("供应商", "Providers"), rows.len(),lang.text("托管账号", "Managed accounts"),if options.accounts { preview.accounts } else { 0 },
        if options.settings { lang.text("恢复应用设置", "Restore preferences") } else { lang.text("保留本机设置", "Keep local preferences") },
        lang.text("尚未恢复。追加 --apply 确认；--no-accounts / --no-settings 可排除相应内容，本机路径默认不恢复。", "Nothing restored. Add --apply; use --no-accounts / --no-settings to exclude scopes. Machine paths are excluded by default."));
    Output::new(
        json!({"preview": preview, "selection": {"tools":options.tools,"accounts":options.accounts,"settings":options.settings,"paths":options.paths,"duplicate":options.duplicate}}),
        format!(
            "{text}\n{}",
            if options.paths {
                lang.text(
                    "已选择恢复备份中的本机目录。",
                    "Machine directory overrides are selected for restore.",
                )
            } else {
                lang.text("保留本机目录。", "Machine directory overrides stay local.")
            }
        ),
    )
}

pub(super) fn report_text(report: &RestoreReport, lang: Language) -> String {
    format!(
        "{}\n{} {} · {} {} · {} {}\n{} {}\n{}",
        if report.failures.is_empty() {
            lang.text("恢复完成", "Restore complete")
        } else {
            lang.text(
                "部分恢复完成，请检查失败项目后重试",
                "Partially restored; review failed stages before retrying",
            )
        },
        lang.text("供应商新增", "Providers added"),
        report.providers_added,
        lang.text("跳过", "Skipped"),
        report.providers_skipped,
        lang.text("账号新增", "Accounts added"),
        report.accounts_added,
        lang.text("应用设置", "Preferences"),
        if report.settings_restored {
            lang.text("已恢复", "restored")
        } else {
            lang.text("未恢复", "not restored")
        },
        if report.failures.is_empty() {
            lang.text(
                "客户端配置未切换。选择供应商后可单独应用。",
                "Client configurations were not switched. Apply a provider separately.",
            )
            .into()
        } else {
            report.failures.join("\n")
        }
    )
}

pub(super) fn export(
    service: &Service,
    path: &Path,
    password_file: Option<&Path>,
    as_json: bool,
    lang: Language,
) -> Result<Output, CliError> {
    let secret = password(password_file, true, as_json, lang)?;
    service.backup_export_file(path, &secret)?;
    Ok(Output::new(
        json!({"path":path,"encrypted":true}),
        format!(
            "{}\n  {}\n{}",
            lang.text("加密备份已保存", "Encrypted backup saved"),
            clean(&path.display().to_string()),
            lang.text(
                "包含供应商、托管账号和设置；请妥善保管密码。",
                "Includes providers, managed accounts and preferences. Keep the passphrase safe."
            )
        ),
    ))
}

pub(super) fn local(
    service: &Service,
    dir: Option<PathBuf>,
    apply: bool,
    accounts: bool,
    tools: Vec<Tool>,
    lang: Language,
) -> Result<Output, CliError> {
    let scan = service.ccswitch_scan(dir)?;
    if !apply {
        return Ok(local_preview(&scan, &tools, accounts, lang));
    }
    let selected = if tools.is_empty() {
        Tool::ALL.to_vec()
    } else {
        tools
    };
    let report = service.ccswitch_apply_options(Some(scan.directory), selected, accounts)?;
    let data = json!({"created":report.created,"accountsAdded":report.accounts_added,"skipped":report.skipped.iter().map(|(tool,id,reason)| json!({"tool":tool,"id":id,"reasonCode":reason.code})).collect::<Vec<_>>()});
    Ok(Output::new(
        data,
        format!(
            "{} {} · {} {} · {} {}\n{}",
            lang.text("已保存卡片", "Cards saved"),
            report.created.len(),
            lang.text("账号", "Accounts"),
            report.accounts_added.len(),
            lang.text("已跳过", "Skipped"),
            report.skipped.len(),
            lang.text(
                "CC Switch 源数据和客户端配置均未修改。",
                "CC Switch source data and client configurations were not modified."
            )
        ),
    ))
}

pub(super) fn restore(
    service: &Service,
    path: &Path,
    password_file: Option<&Path>,
    apply: bool,
    options: RestoreOptions,
    as_json: bool,
    lang: Language,
) -> Result<Output, CliError> {
    let secret = password(password_file, false, as_json, lang)?;
    let bytes = read_limited(path, MAX_BACKUP_BYTES, false)?;
    let preview = service.backup_load(&bytes, &secret)?;
    if !apply {
        return Ok(backup_preview(&preview, &options, lang));
    }
    let report = service.backup_apply(options)?;
    if !report.failures.is_empty() {
        return Err(CliError { code:"backup.partial".into(), message:"Restore completed only in part. Already imported data is retained. Inspect providers, accounts and settings before retrying.", exit:1 });
    }
    let text = report_text(&report, lang);
    Ok(Output::new(json!(report), text))
}
