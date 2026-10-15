//! `leanflip scene list | show | apply`. Scenes are created and edited in the App;
//! the terminal lists, previews and applies them. Like the other commands, output
//! carries card names and ids only, never configuration or backend error text.
use leanflip_core::scene::Action;
use leanflip_core::Tool;
use leanflip_service::service::{SceneOutcome, SceneStatus, ToolPreview};
use leanflip_service::Service;
use serde_json::{json, Value};

use crate::args::{Language, SceneCommand, ToolArg};
use crate::presentation::{clean, table};
use crate::{CliError, Output};

pub fn run(service: &Service, action: SceneCommand, lang: Language) -> Result<Output, CliError> {
    match action {
        SceneCommand::List => list(service, lang),
        SceneCommand::Show { scene } => show(service, &scene, lang),
        SceneCommand::Apply { scene, tools } => apply(service, &scene, &tools, lang),
    }
}

fn status_text(status: SceneStatus, lang: Language) -> &'static str {
    match status {
        SceneStatus::Active => lang.text("当前一致", "Matches"),
        SceneStatus::Drifted => lang.text("已偏离", "Drifted"),
        SceneStatus::Idle => lang.text("未应用", "Not applied"),
    }
}

fn tool_names(tools: impl IntoIterator<Item = Tool>) -> String {
    tools
        .into_iter()
        .map(Tool::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn list(service: &Service, lang: Language) -> Result<Output, CliError> {
    let list = service.scenes()?;
    let mut text = format!(
        "{} · {} {}",
        lang.text("LeanFlip · 场景", "LeanFlip · Scenes"),
        list.scenes.len(),
        lang.text("个", "scene(s)")
    );
    if list.scenes.is_empty() {
        text.push_str(lang.text(
            "\n\n还没有场景。在 LeanFlip App 首页点「场景」，把当前状态存为场景。",
            "\n\nNo scenes yet. In the LeanFlip App, open Scenes on Home and save the current setup as a scene.",
        ));
    } else {
        let rows = list
            .scenes
            .iter()
            .map(|view| {
                let mut status = status_text(view.status, lang).to_string();
                if !view.missing.is_empty() {
                    status.push_str(&format!(
                        "{}{} {}",
                        lang.text("，缺 ", ", "),
                        view.missing.len(),
                        lang.text("个供应商", "missing provider(s)")
                    ));
                }
                vec![
                    clean(&view.scene.name),
                    status,
                    tool_names(view.scene.entries().into_iter().map(|(tool, _)| tool)),
                ]
            })
            .collect::<Vec<_>>();
        text.push_str(&format!(
            "\n\n{}\n\n{}\n  leanflip scene show <{}>\n  leanflip scene apply <{}>",
            table(
                &[
                    lang.text("名称", "NAME"),
                    lang.text("状态", "STATE"),
                    lang.text("工具", "TOOLS")
                ],
                &rows
            ),
            lang.text("下一步：", "Next:"),
            lang.text("名称", "name"),
            lang.text("名称", "name"),
        ));
    }
    let scenes = list
        .scenes
        .iter()
        .map(|view| {
            json!({
                "id": view.scene.id, "name": view.scene.name, "status": view.status,
                "tools": view.scene.entries().into_iter().map(|(tool, _)| tool).collect::<Vec<_>>(),
                "pending": view.pending, "missing": view.missing,
            })
        })
        .collect::<Vec<_>>();
    Ok(Output::new(
        json!({"scenes": scenes, "last": list.last}),
        text,
    ))
}

/// One line per tool: what applying the scene would do.
fn change_text(tool: &ToolPreview, lang: Language) -> String {
    if tool.error.is_some() {
        return lang
            .text(
                "客户端配置无法读取，不会改动；运行 leanflip paths 检查",
                "Client configuration is unreadable and will be left alone; run leanflip paths to inspect it",
            )
            .into();
    }
    let names = |want: fn(&Action) -> bool| {
        tool.steps
            .iter()
            .filter(|step| want(&step.action))
            .map(|step| clean(&step.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    let switch = names(|a| matches!(a, Action::Switch(_)));
    if !switch.is_empty() {
        let from = tool
            .from
            .as_deref()
            .map(clean)
            .unwrap_or_else(|| lang.text("未选择", "none").into());
        parts.push(format!("{from} → {switch}"));
    }
    let activate = names(|a| matches!(a, Action::Activate(_)));
    if !activate.is_empty() {
        parts.push(format!(
            "{} {activate}",
            lang.text("默认改为", "default to")
        ));
    }
    let enable = names(|a| matches!(a, Action::Enable(_)));
    if !enable.is_empty() {
        parts.push(format!("{} {enable}", lang.text("接入", "add")));
    }
    let disable = names(|a| matches!(a, Action::Disable(_)));
    if !disable.is_empty() {
        parts.push(format!("{} {disable}", lang.text("移出", "remove")));
    }
    if !tool.missing.is_empty() {
        parts.push(format!(
            "{} {}",
            lang.text("跳过已删除的", "skip deleted"),
            tool.missing
                .iter()
                .map(|id| clean(id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if parts.is_empty() {
        lang.text("已一致", "Already matches").into()
    } else {
        parts.join(" · ")
    }
}

fn preview_json(tool: &ToolPreview) -> Value {
    json!({
        "tool": tool.tool, "kind": tool.kind, "from": tool.from,
        "steps": tool.steps, "missing": tool.missing, "readable": tool.error.is_none(),
    })
}

fn show(service: &Service, key: &str, lang: Language) -> Result<Output, CliError> {
    let preview = service.scene_preview(key)?;
    let rows = preview
        .tools
        .iter()
        .map(|tool| vec![tool.tool.to_string(), change_text(tool, lang)])
        .collect::<Vec<_>>();
    let text = format!(
        "{} · {}\n{}\n\n{}\n\n{}\n  leanflip scene apply {}",
        lang.text("场景预览", "Scene preview"),
        clean(&preview.scene.name),
        lang.text("尚未改动任何配置。", "Nothing has been changed."),
        table(
            &[lang.text("工具", "TOOL"), lang.text("变化", "CHANGE")],
            &rows
        ),
        lang.text("应用：", "To apply:"),
        preview.scene.id,
    );
    Ok(Output::new(
        json!({"scene": {"id": preview.scene.id, "name": preview.scene.name},
               "tools": preview.tools.iter().map(preview_json).collect::<Vec<_>>()}),
        text,
    ))
}

fn apply(
    service: &Service,
    key: &str,
    tools: &[ToolArg],
    lang: Language,
) -> Result<Output, CliError> {
    let scene = service.scene_preview(key)?.scene;
    let included = scene
        .entries()
        .into_iter()
        .map(|(tool, _)| tool)
        .collect::<Vec<_>>();
    let only = tools
        .iter()
        .map(|&tool| Tool::from(tool))
        .collect::<Vec<_>>();
    if only.iter().any(|tool| !included.contains(tool)) {
        return Err(CliError::input(
            "scene.tool_not_included",
            "A requested tool is not part of this scene. Run leanflip scene show <scene> to see its tools.",
        ));
    }
    let report = service.apply_scene(&scene.id, (!only.is_empty()).then_some(only))?;
    let rows = report
        .results
        .iter()
        .map(|result| {
            let outcome = match result.outcome {
                SceneOutcome::Applied => lang.text("已切换", "Switched"),
                SceneOutcome::Unchanged => lang.text("无需改动", "No change needed"),
                SceneOutcome::Failed => lang.text("失败", "Failed"),
            };
            let mut detail = Vec::new();
            if result.outcome == SceneOutcome::Failed {
                detail.push(format!("leanflip paths {}", result.tool));
            }
            if !result.missing.is_empty() {
                detail.push(format!(
                    "{} {}",
                    lang.text("跳过已删除的", "skipped deleted"),
                    result
                        .missing
                        .iter()
                        .map(|id| clean(id))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !result.warnings.is_empty() {
                detail.push(format!(
                    "{} {}",
                    lang.text("配置警告", "warnings"),
                    result.warnings.len()
                ));
            }
            vec![result.tool.to_string(), outcome.into(), detail.join(" · ")]
        })
        .collect::<Vec<_>>();
    let failed = report.failed();
    let text = format!(
        "{} · {}\n\n{}",
        if failed {
            lang.text("场景只应用了一部分", "Scene partly applied")
        } else {
            lang.text("场景已应用", "Scene applied")
        },
        clean(&scene.name),
        table(
            &[
                lang.text("工具", "TOOL"),
                lang.text("结果", "RESULT"),
                lang.text("说明", "DETAIL")
            ],
            &rows
        ),
    );
    let results = report
        .results
        .iter()
        .map(|result| {
            json!({
                "tool": result.tool, "outcome": result.outcome, "changed": result.changed,
                "warningCount": result.warnings.len(), "missing": result.missing,
            })
        })
        .collect::<Vec<_>>();
    let mut output = Output::new(
        json!({"scene": {"id": scene.id, "name": scene.name}, "results": results}),
        text,
    );
    if failed {
        output.failure = Some(CliError {
            code: "scene.partial".into(),
            message: "The scene was only partly applied; tools that failed were left unchanged. Inspect them with leanflip paths <tool> and retry.",
            exit: 1,
        });
    }
    Ok(output)
}
