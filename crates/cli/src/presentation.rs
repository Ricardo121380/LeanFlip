//! Human-facing output only. Machine data and error messages remain unchanged.
use std::ffi::OsString;
use std::io::IsTerminal;

/// Styling only at the terminal boundary; pipes and JSON retain plain output.
pub fn human_output(text: &str) -> String {
    if !std::io::stdout().is_terminal()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").as_deref() == Ok("dumb")
    {
        return text.into();
    }
    // Only the first line is emphasised; no banner, so repeated commands stay compact.
    let (heading, rest) = text.split_once('\n').unwrap_or((text, ""));
    if rest.is_empty() {
        format!("\x1b[1m{heading}\x1b[0m")
    } else {
        format!("\x1b[1m{heading}\x1b[0m\n{rest}")
    }
}

use unicode_width::UnicodeWidthStr;

use crate::args::Language;

pub fn clean(value: &str) -> String {
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

/// Narrow terminals use labelled records instead of clipping IDs needed by later commands.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let preferred = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|width| (40..=240).contains(width));
    let width = if std::io::stdout().is_terminal() {
        let actual = dialoguer::console::Term::stdout().size().1.max(1) as usize;
        preferred.map_or(actual, |preferred| preferred.min(actual))
    } else {
        preferred.unwrap_or(100)
    };
    table_at_width(headers, rows, width)
}

fn table_at_width(headers: &[&str], rows: &[Vec<String>], terminal_width: usize) -> String {
    let rows = rows
        .iter()
        .map(|row| row.iter().map(|v| clean(v)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let widths = headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            rows.iter()
                .map(|row| row[index].width())
                .chain(std::iter::once(header.width()))
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    if widths.iter().sum::<usize>() + headers.len().saturating_sub(1) * 2 > terminal_width {
        let label_width = headers
            .iter()
            .map(|header| header.width())
            .max()
            .unwrap_or(0);
        return rows
            .iter()
            .map(|row| {
                headers
                    .iter()
                    .zip(row)
                    .map(|(header, value)| {
                        format!(
                            "{header}{}  {value}",
                            " ".repeat(label_width.saturating_sub(header.width()))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    let render = |row: &[String]| {
        row.iter()
            .enumerate()
            .map(|(index, value)| {
                if index + 1 == row.len() {
                    value.clone()
                } else {
                    format!(
                        "{value}{}",
                        " ".repeat(widths[index].saturating_sub(value.width()) + 2)
                    )
                }
            })
            .collect::<String>()
    };
    std::iter::once(render(
        &headers.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    ))
    .chain(std::iter::once(
        widths
            .iter()
            .map(|width| "─".repeat(*width))
            .collect::<Vec<_>>()
            .join("  "),
    ))
    .chain(rows.iter().map(|row| render(row)))
    .collect::<Vec<_>>()
    .join("\n")
}

/// Only select the presentation language before clap handles help/errors. Clap
/// remains responsible for all parsing and validation; never echo invalid input.
pub fn language_before_parse(args: &[OsString]) -> Language {
    let mut lang = Language::Zh;
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        let value = if arg == "--lang" {
            args.next().and_then(|s| s.to_str())
        } else {
            arg.to_str().and_then(|s| s.strip_prefix("--lang="))
        };
        match value {
            Some("en") => lang = Language::En,
            Some("zh") => lang = Language::Zh,
            _ => {}
        }
    }
    lang
}

pub fn help(mut command: clap::Command, lang: Language) -> clap::Command {
    // Materialize clap's generated help/version arguments before translating them.
    command.build();
    localize_help(&mut command, "", lang);
    command
}

/// Select help only from the known command tree. The partial matches are used
/// for presentation after a failed parse, never to execute an operation.
pub fn matched_help(command: &clap::Command, matches: Option<&clap::ArgMatches>) -> clap::Command {
    let mut command = command;
    let mut matches = matches;
    while let Some((name, submatches)) = matches.and_then(clap::ArgMatches::subcommand) {
        let Some(subcommand) = command.find_subcommand(name) else {
            break;
        };
        command = subcommand;
        matches = Some(submatches);
    }
    command.clone()
}

fn localize_help(command: &mut clap::Command, path: &str, lang: Language) {
    let (zh, examples) = match path {
        "" => ("管理 AI 编程工具的供应商配置和 Codex 账号", "leanflip tui\n  leanflip menu\n  leanflip status\n  leanflip list codex\n  leanflip use codex <provider-id>\n  leanflip enable opencode <provider-id>\n  leanflip status --json\n  leanflip --lang en --help"),
        "menu" => ("交互菜单：搜索并选择工具、供应商，回车执行", "leanflip menu\n  leanflip menu codex\n  leanflip menu --lang en"),
        "tui" => ("全屏终端工作台：浏览工具、搜索供应商、确认切换和查看路径", "leanflip tui\n  leanflip tui codex\n  leanflip tui --lang en\n  leanflip --home /tmp/leanflip-demo tui"),
        "status" => ("查看各工具已保存的选择及配置检测状态", "leanflip status\n  leanflip status --json"),
        "list" => ("列出供应商及当前状态，隐藏配置内容和凭据", "leanflip list codex\n  leanflip list opencode --json"),
        "use" => ("切换供应商；仅适用于切换式工具", "leanflip list codex\n  leanflip use codex <provider-id>"),
        "enable" => ("将供应商写入并存式工具的客户端配置", "leanflip enable opencode <provider-id>"),
        "disable" => ("从客户端配置移出供应商，供应商本身仍保留", "leanflip disable opencode <provider-id>"),
        "paths" => ("查看实际生效的配置路径，包含 App 目录覆盖", "leanflip paths codex\n  leanflip --home /tmp/leanflip-demo paths"),
        "export" => ("导出包含供应商、托管账号和设置的密码加密备份", "leanflip export backup.leanflip\n  leanflip export backup.leanflip --password-file private-passphrase.txt"),
        "import local" => ("自动检测本机 CC Switch；默认预览，--apply 才迁入", "leanflip import local\n  leanflip import local --accounts --apply\n  leanflip import local --dir /absolute/path/.cc-switch"),
        "import backup" => ("解密并预览完整备份；--apply 恢复所选内容", "leanflip import backup backup.leanflip\n  leanflip import backup backup.leanflip --apply\n  leanflip import backup backup.leanflip --no-providers --no-settings --apply"),
        "import" => ("导入客户端现有配置或 cc-switch 导出文件", "leanflip import live claude\n  leanflip import file backup.sql\n  leanflip import file backup.sql --apply --tool codex"),
        "import live" => ("将客户端当前配置导入为供应商", "leanflip import live claude\n  leanflip list claude"),
        "import file" => ("预览 SQL、SQLite 或旧版 JSON 导出；--apply 才会保存", "leanflip import file backup.sql\n  leanflip import file backup.sql --apply --tool codex\n  leanflip import file backup.sql --apply --duplicate rename"),
        "provider edit" => ("交互编辑名称、Base URL 与 API Key；保存前预览影响范围", "leanflip provider edit claude <provider-id>\n  leanflip provider edit codex <provider-id>"),
        "provider" => ("新增、更新或删除供应商", "leanflip provider add claude --file provider.json\n  leanflip provider update claude --file provider.json\n  leanflip provider delete claude <provider-id>"),
        "provider add" => ("保存新供应商；--activate 可同时应用到客户端", "leanflip provider add claude --file provider.json\n  cat provider.json | leanflip provider add claude"),
        "provider update" => ("按 JSON 中的 id 替换供应商；生效中的配置也会更新", "leanflip provider update claude --file provider.json"),
        "provider delete" => ("删除供应商；正在使用的切换式供应商需先切换走", "leanflip list claude\n  leanflip provider delete claude <provider-id>"),
        "accounts" => ("管理 Codex 的 ChatGPT 托管账号", "leanflip accounts list\n  leanflip accounts login\n  leanflip accounts default <account-id>"),
        "accounts list" => ("列出账号身份和登录状态，隐藏 token", "leanflip accounts list\n  leanflip accounts list --json"),
        "accounts login" => ("通过设备验证码登录；按提示在浏览器中确认", "leanflip accounts login\n  leanflip accounts login --account <account-id> --timeout 600"),
        "accounts default" => ("设置默认托管账号", "leanflip accounts default <account-id>"),
        "accounts remove" => ("删除本地托管账号，不撤销 OpenAI 服务端登录", "leanflip accounts remove <account-id>"),
        "accounts refresh" => ("强制刷新账号凭据，不输出 token", "leanflip accounts refresh <account-id>"),
        "recover" => ("显式处理未完成的写入；查询命令不会自动恢复", "leanflip recover codex\n  leanflip recover"),
        "quota" => ("查看 Codex 和中转站的额度；默认只读本机记录，--refresh 联网查询", "leanflip quota\n  leanflip quota --refresh\n  leanflip quota --json"),
        "scene" => ("列出、预览或应用场景；场景在 App 中创建和编辑", "leanflip scene list\n  leanflip scene show work\n  leanflip scene apply work"),
        "scene list" => ("列出场景，以及是否和当前配置一致", "leanflip scene list\n  leanflip scene list --json"),
        "scene show" => ("预览应用场景会改哪些工具，不写入", "leanflip scene show work"),
        "scene apply" => ("应用场景；--tool 只应用其中的部分工具", "leanflip scene apply work\n  leanflip scene apply work --tool claude,codex"),
        _ => ("显示命令帮助", "leanflip --help"),
    };
    let examples = format!("{}\n  {examples}", lang.text("示例：", "Examples:"));
    *command = std::mem::take(command).after_help(examples);
    if lang == Language::Zh {
        *command = std::mem::take(command)
            .about(zh)
            .subcommand_help_heading("命令")
            .help_template("{about-with-newline}\n用法： {usage}\n\n{all-args}{after-help}")
            .mut_args(|arg| {
                let description = match arg.get_id().as_str() {
                    "json" => Some("输出 JSON；错误写入标准错误，保留稳定错误码"),
                    "home" => Some("使用指定 HOME 布局；仍应用该目录下的 App 目录覆盖"),
                    "lang" => Some("终端输出与帮助的语言；不影响 JSON 消息"),
                    "tool" => Some("目标工具"),
                    "tools" => Some("仅处理这些工具；可重复指定或以逗号分隔"),
                    "provider_id" => Some("供应商 ID，可通过 leanflip list 查看"),
                    "account_id" => Some("账号 ID，可通过 leanflip accounts list 查看"),
                    "scene" => Some("场景名称或 ID，可通过 leanflip scene list 查看"),
                    "path" => Some("备份或导出文件路径"),
                    "dir" => Some("指定 CC Switch 数据目录的绝对路径"),
                    "password_file" => Some("从私有文件读取密码；省略时隐藏输入，不接受密码值参数"),
                    "accounts" => Some("同时迁入托管 Codex 账号与登录凭据"),
                    "no_accounts" => Some("不恢复托管账号"),
                    "no_providers" => Some("不恢复供应商"),
                    "no_settings" => Some("不恢复应用偏好"),
                    "restore_paths" => Some("同时恢复本机目录覆盖；默认保留目标设备路径"),
                    "file" => Some("供应商 JSON 文件；- 表示标准输入。不要将密钥放入命令参数"),
                    "preview" => Some("仅预览，不保存（默认行为）"),
                    "apply" => Some("确认写入所选数据，不改变客户端当前配置"),
                    "duplicate" => Some("同 ID 供应商的处理方式：skip 跳过，rename 改名"),
                    "activate" => Some("保存后立即应用到客户端"),
                    "account" => Some("重新认证已有账号"),
                    "refresh" => Some("联网查询托管账号和已设置平台的中转卡，并保存结果"),
                    "timeout" => Some("等待登录的秒数；1–1800，单次网络请求另有超时"),
                    "help" => Some("显示帮助"),
                    "version" => Some("显示版本"),
                    _ => None,
                };
                let positional = arg.is_positional();
                let arg = if let Some(description) = description {
                    arg.help(description)
                } else {
                    arg
                };
                arg.help_heading(if positional { "参数" } else { "选项" })
            });
    }
    for sub in command.get_subcommands_mut() {
        let subpath = if path.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{path} {}", sub.get_name())
        };
        localize_help(sub, &subpath, lang);
    }
}

pub fn reason(code: &str, lang: Language) -> &str {
    match code {
        "unsupported_app" => lang.text("暂不支持此工具", "Unsupported tool"),
        "malformed" => lang.text("缺少 ID 或配置", "Missing ID or settings"),
        "plugin" => lang.text("插件配置，不是供应商", "Plugin configuration"),
        "routing" => lang.text(
            "需要代理或协议转换",
            "Requires a proxy or protocol conversion",
        ),
        "unsupported_config" => lang.text("配置未通过兼容检查", "Unsupported configuration"),
        "same_config" => lang.text("已有相同配置", "Identical configuration exists"),
        "same_id" => lang.text(
            "已有相同 ID；可用 --duplicate rename",
            "ID exists; try --duplicate rename",
        ),
        "renamed" => lang.text("使用新的 ID 导入", "Import with a new ID"),
        "account_binding" => lang.text(
            "切换前需先登录绑定账号",
            "Sign in to the bound account before switching",
        ),
        _ => lang.text("请检查导入项", "Review this item"),
    }
}

pub fn error(code: &str, fallback: &str, lang: Language) -> String {
    let zh = match code {
        "provider.connection_invalid" => lang.text("名称不能为空；Base URL 需要使用不带凭据、查询参数或片段的 HTTP(S) 地址，API Key 不能包含控制字符。", "Use a non-empty name and an HTTP(S) Base URL without credentials, query or fragment. API keys cannot contain control characters."),
        "provider.connection_unsupported" => lang.text("此供应商使用官方登录或高级配置，请在 App 中管理连接字段。", "This provider uses official login or advanced configuration. Manage its connection fields in the App."),
        "backup.password" => lang.text("密码需要 10–4096 个字符。", "Use a passphrase of 10–4096 characters."),
        "backup.password_required" => lang.text("在交互终端输入密码，或使用 --password-file 指定私有密码文件。", "Use an interactive terminal or --password-file with a private passphrase file."),
        "backup.decrypt" => lang.text("密码不正确或备份已损坏；没有恢复数据。检查密码后重试。", "Incorrect passphrase or damaged backup. Nothing restored; check the passphrase and retry."),
        "backup.exists" => lang.text("备份目标已存在，请使用新的文件名。", "The destination exists. Choose a new backup filename."),
        "backup.invalid" => lang.text("备份格式、大小或版本不受支持。", "Unsupported backup format, size or version."),
        "backup.partial" => lang.text("仅部分数据恢复成功。已写入的内容保留，请检查后重试。", "Only part of the data was restored. Completed changes remain; inspect and retry."),
        "invalid_arguments" => "命令参数不正确。运行 leanflip --help，或 leanflip <command> --help 查看用法。",
        "interactive_terminal" => "交互界面需要真实终端，不能与管道或 --json 一起使用。运行 leanflip --help 查看命令用法。",
        "terminal_io" => "交互终端无法读写，请重新打开终端，或用 leanflip --help 中的普通命令继续操作。",
        "interrupted" => "已中断。",
        "invalid_tool_mode" => "命令与工具模式不匹配。切换式工具使用 use；并存式工具使用 enable / disable。",
        "invalid_provider_json" => "供应商 JSON 无效。请检查 id、name、settingsConfig 字段；输入原文已隐藏。",
        "invalid_input" => "输入或当前操作不适用。请核对供应商 JSON；删除正在使用的供应商前需先切换走。",
        "input_too_large" => "输入文件超过大小限制。供应商 JSON 上限为 1 MiB，导出文件上限为 256 MiB。",
        "not_found" => "找不到指定项目。用 leanflip list <tool> 核对供应商 ID，或 leanflip accounts list 核对账号 ID。",
        "config" | "json" => "配置无法读取或校验。运行 leanflip paths <tool> 定位文件，检查格式后重试。",
        "io" => "无法访问所需文件。请核对输入文件路径，并用 leanflip paths <tool> 检查配置路径与权限。",
        "conflict" => "配置被其他程序修改。请重新查看当前状态后重试，不要覆盖其他程序的新配置。",
        "serialization" => "配置序列化失败，本次操作未完成。请检查输入配置。",
        "transport_init" => "无法初始化登录网络通道。请检查网络或代理设置后重试。",
        "login_timeout" | "codex.oauth.device_expired" => "登录码已过期或等待超时。运行 leanflip accounts login 获取新验证码。",
        "codex.oauth.account_not_found" => "找不到托管账号。运行 leanflip accounts list 核对 ID，或 leanflip accounts login 添加账号。",
        "codex.oauth.reauth_required" | "codex.oauth.refresh_rejected" | "codex.oauth.ambiguous_generation" => "此账号需要重新登录。运行 leanflip accounts login --account <account-id>，避免使用旧凭据。",
        "codex.oauth.reauth_workspace_mismatch" | "codex.oauth.reauth_user_mismatch" | "codex.oauth.workspace_mismatch" => "登录身份或工作区与目标账号不匹配。请在浏览器中选择对应账号和工作区后重新登录。",
        "codex.oauth.duplicate_account" => "该账号已存在。用 leanflip accounts list 查看已有账号，或使用 login --account 重新认证。",
        "codex.oauth.live_sync_failed" => "新凭据已保存在账号库，但 Codex 客户端同步失败。运行 leanflip paths codex 检查路径与权限；不要用旧 token 覆盖新凭据。",
        "scene.not_found" => "找不到这个场景。运行 leanflip scene list 查看场景名称。",
        "scene.invalid" => "场景内容无效。请在 LeanFlip App 中编辑这个场景后重试。",
        "scene.partial" => "场景只应用了一部分。失败的工具已尽量改回原样，标了「可能改了一部分」的请先检查；运行 leanflip paths <tool> 查看配置后重试。",
        "scene.tool_not_included" => "指定的工具不在这个场景里。运行 leanflip scene show <场景> 查看包含的工具。",
        _ if code.starts_with("ccswitch_export.") => "无法导入该文件。请使用 cc-switch 导出的 SQL、SQLite 或旧版 JSON，并先运行 import file 预览。",
        _ => "操作未完成。请根据错误码检查相关配置；配置原文和凭据不会在终端显示。",
    };
    let mut text = format!(
        "{} [{code}]\n{}",
        lang.text("错误", "Error"),
        lang.text(zh, fallback)
    );
    if lang == Language::En {
        let hint = match code {
            "not_found" => Some("Next: leanflip list <tool> or leanflip accounts list"),
            "invalid_input" => {
                Some("Check provider JSON; switch away before deleting the current provider.")
            }
            "config" | "json" | "io" | "codex.oauth.live_sync_failed" => {
                Some("Next: leanflip paths <tool>")
            }
            "codex.oauth.reauth_required"
            | "codex.oauth.refresh_rejected"
            | "codex.oauth.ambiguous_generation" => {
                Some("Next: leanflip accounts login --account <account-id>")
            }
            "codex.oauth.account_not_found" => {
                Some("Next: leanflip accounts list or leanflip accounts login")
            }
            "codex.oauth.device_expired" => Some("Next: leanflip accounts login"),
            "scene.not_found" => Some("Next: leanflip scene list"),
            _ => None,
        };
        if let Some(hint) = hint {
            text.push_str(&format!("\n{hint}"));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_aligns_cjk_names_by_display_width() {
        let text = table_at_width(
            &["名称", "状态"],
            &[
                vec!["中文服务".into(), "当前".into()],
                vec!["English".into(), "待用".into()],
            ],
            100,
        );
        let rows = text.lines().skip(2).collect::<Vec<_>>();
        assert_eq!(
            rows[0].split("当前").next().unwrap().width(),
            rows[1].split("待用").next().unwrap().width()
        );
    }

    #[test]
    fn narrow_table_preserves_full_identifiers_and_escapes_controls() {
        let id = "very-long-provider-id-which-must-stay-copyable";
        let text = table_at_width(
            &["ID", "名称"],
            &[vec![id.into(), "供应商\u{1b}[2J".into()]],
            40,
        );
        assert!(text.contains(id));
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("\\u{1b}"));
        assert_eq!(text.lines().count(), 2);
    }
}
