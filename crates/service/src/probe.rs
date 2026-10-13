//! 供应商检测：测连通、获取模型列表、检查模型。只在用户点击时运行，从不自动发起。
//!
//! - 测连通只访问请求地址，不带任何凭据；收到任何 HTTP 响应都算连通。
//! - 获取模型列表和检查模型会把密钥发往配置里的地址，和客户端自己发出的请求一致。
//! - 不跟随重定向，免得把密钥带到别的主机；返回给界面的错误说明里遮掉密钥原文。
use std::io::Read;
use std::time::{Duration, Instant};

use leanflip_core::Tool;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};
use toml_edit::{DocumentMut, Item};

use crate::oauth_http::USER_AGENT;

const LATENCY_TIMEOUT: Duration = Duration::from_secs(8);
const MODELS_TIMEOUT: Duration = Duration::from_secs(20);
const CHECK_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_BODY: u64 = 8 * 1024 * 1024;
const MAX_PAGES: usize = 5;
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Responses 接口要求 max_output_tokens ≥ 16；其他协议也用这个值，开销同样很小。
const CHECK_TOKENS: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Protocol {
    /// 地址不含 /v1，请求 `{base}/v1/messages`（Claude Code、Anthropic SDK）。
    Anthropic,
    /// 地址已含 /v1，请求 `{base}/messages`（OpenCode 的 @ai-sdk/anthropic）。
    AnthropicV1,
    OpenAiChat,
    OpenAiResponses,
    Gemini,
}

#[derive(Debug, PartialEq)]
enum Auth {
    None,
    Bearer(String),
    ApiKey(String),
    Google(String),
}

#[derive(Debug)]
struct Target {
    protocol: Protocol,
    base: Url,
    auth: Auth,
    model: Option<String>,
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
}

impl Target {
    fn secrets(&self) -> Vec<&str> {
        let mut secrets: Vec<&str> = self.headers.iter().map(|(_, v)| v.as_str()).collect();
        if let Auth::Bearer(key) | Auth::ApiKey(key) | Auth::Google(key) = &self.auth {
            secrets.push(key);
        }
        secrets.retain(|s| s.len() >= 4);
        secrets
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 模型列表自带的推理强度档位（目前只有 Anthropic 的 capabilities.effort 提供）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
}

/// 检测可用档位时每一档的结果。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelResult {
    pub level: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

const ANTHROPIC_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const OPENAI_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: u64,
    /// 失败类别（no_base_url / auth / timeout …），成功时为空。
    pub code: Option<String>,
    /// 供应商返回的错误说明、缺少的环境变量名等，已遮掉密钥。
    pub detail: Option<String>,
    /// 实际请求的地址，不含查询参数和账号信息。
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub models: Option<Vec<RemoteModel>>,
    pub levels: Option<Vec<LevelResult>>,
}

#[derive(Debug, PartialEq)]
struct Failure {
    code: &'static str,
    detail: Option<String>,
    status: Option<u16>,
}

impl Failure {
    fn new(code: &'static str) -> Self {
        Self {
            code,
            detail: None,
            status: None,
        }
    }
    fn detail(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
            status: None,
        }
    }
}

fn finish(endpoint: Option<&Url>, result: Result<ProbeResult, Failure>) -> ProbeResult {
    let mut result = result.unwrap_or_else(|failure| ProbeResult {
        status: failure.status,
        code: Some(failure.code.into()),
        detail: failure.detail,
        ..ProbeResult::default()
    });
    if result.endpoint.is_none() {
        result.endpoint = endpoint.map(display);
    }
    result
}

/// 测连通：GET 请求地址本身，不带凭据。超时重试一次。
pub fn latency(tool: Tool, config: &Value) -> ProbeResult {
    match (target(tool, config), client(LATENCY_TIMEOUT)) {
        (Ok(target), Ok(client)) => latency_with(&client, &target),
        (Err(failure), _) | (_, Err(failure)) => finish(None, Err(failure)),
    }
}

/// 获取模型列表：按协议请求 models 接口，最多翻 5 页。
pub fn models(tool: Tool, config: &Value) -> ProbeResult {
    match (target(tool, config), client(MODELS_TIMEOUT)) {
        (Ok(target), Ok(client)) => models_with(&client, &target),
        (Err(failure), _) | (_, Err(failure)) => finish(None, Err(failure)),
    }
}

/// 检查模型：发送一次最短的真实请求（最多 16 个输出 token）。
pub fn check(tool: Tool, config: &Value, model: Option<&str>) -> ProbeResult {
    match (target(tool, config), client(CHECK_TIMEOUT)) {
        (Ok(target), Ok(client)) => check_with(&client, &target, model),
        (Err(failure), _) | (_, Err(failure)) => finish(None, Err(failure)),
    }
}

/// 检测可用的推理强度：先发一次不带档位的请求确认连接和模型可用，再逐档各发一次。
/// 被 400 拒绝的档位视为不支持；其他错误中止检测。
pub fn efforts(tool: Tool, config: &Value, model: Option<&str>) -> ProbeResult {
    match (target(tool, config), client(CHECK_TIMEOUT)) {
        (Ok(target), Ok(client)) => efforts_with(&client, &target, model),
        (Err(failure), _) | (_, Err(failure)) => finish(None, Err(failure)),
    }
}

fn client(timeout: Duration) -> Result<Client, Failure> {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(timeout.min(Duration::from_secs(8)))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Failure::new("network"))
}

fn latency_with(client: &Client, target: &Target) -> ProbeResult {
    let url = target.base.clone();
    let attempt = || send(client.get(url.clone()));
    let result = match attempt() {
        Err(failure) if failure.code == "timeout" => attempt(),
        other => other,
    };
    finish(
        Some(&url),
        result.map(|(response, ms)| ProbeResult {
            ok: true,
            status: Some(response.status().as_u16()),
            latency_ms: ms,
            ..ProbeResult::default()
        }),
    )
}

fn models_with(client: &Client, target: &Target) -> ProbeResult {
    let first = models_url(target);
    let run = || -> Result<ProbeResult, Failure> {
        let mut models: Vec<RemoteModel> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut total = 0;
        for _ in 0..MAX_PAGES {
            let mut url = models_url(target);
            match target.protocol {
                Protocol::Anthropic | Protocol::AnthropicV1 => {
                    url.query_pairs_mut().append_pair("limit", "1000");
                    if let Some(cursor) = &cursor {
                        url.query_pairs_mut().append_pair("after_id", cursor);
                    }
                }
                Protocol::Gemini => {
                    url.query_pairs_mut().append_pair("pageSize", "1000");
                    if let Some(cursor) = &cursor {
                        url.query_pairs_mut().append_pair("pageToken", cursor);
                    }
                }
                _ => {}
            }
            let (response, ms) = send(authorize(target, client.get(url)))?;
            total += ms;
            let (status, body) = read(response)?;
            if !(200..300).contains(&status) {
                return Err(http_failure(status, &body, &target.secrets()));
            }
            let value: Value =
                serde_json::from_slice(&body).map_err(|_| Failure::new("bad_response"))?;
            let (page, next) = parse_models(target.protocol, &value)
                .ok_or_else(|| Failure::new("bad_response"))?;
            for model in page {
                if !models.iter().any(|m| m.id == model.id) {
                    models.push(model);
                }
            }
            match next {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => break,
            }
        }
        Ok(ProbeResult {
            ok: true,
            status: Some(200),
            latency_ms: total,
            models: Some(models),
            ..ProbeResult::default()
        })
    };
    finish(Some(&first), run())
}

fn chosen_model(target: &Target, model: Option<&str>) -> Option<String> {
    model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(String::from)
        .or_else(|| target.model.clone())
}

/// 发一次最短请求，返回状态码、响应体和耗时。新的 OpenAI 推理模型不接受 max_tokens，
/// 遇到这种拒绝时换成 max_completion_tokens 再试一次。
fn minimal(
    client: &Client,
    target: &Target,
    url: &Url,
    model: &str,
    effort: Option<&str>,
) -> Result<(u16, Vec<u8>, u64), Failure> {
    let mut limit = "max_tokens";
    loop {
        let body = check_body(target.protocol, model, limit, effort);
        let (response, ms) = send(authorize(target, client.post(url.clone()).json(&body)))?;
        let (status, bytes) = read(response)?;
        if status == 400
            && target.protocol == Protocol::OpenAiChat
            && limit == "max_tokens"
            && String::from_utf8_lossy(&bytes).contains("max_completion_tokens")
        {
            limit = "max_completion_tokens";
            continue;
        }
        return Ok((status, bytes, ms));
    }
}

/// 2xx 且不是网关包装的错误对象才算成功。
fn accepted(status: u16, bytes: &[u8], target: &Target) -> Result<(), Failure> {
    if !(200..300).contains(&status) {
        return Err(http_failure(status, bytes, &target.secrets()));
    }
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        if value.get("error").is_some_and(|e| !e.is_null()) {
            let mut failure = http_failure(status, bytes, &target.secrets());
            failure.code = "bad_response";
            return Err(failure);
        }
    }
    Ok(())
}

fn check_with(client: &Client, target: &Target, model: Option<&str>) -> ProbeResult {
    let Some(model) = chosen_model(target, model) else {
        return finish(None, Err(Failure::new("no_model")));
    };
    let url = check_url(target, &model);
    let run = || -> Result<ProbeResult, Failure> {
        let (status, bytes, ms) = minimal(client, target, &url, &model, None)?;
        accepted(status, &bytes, target)?;
        Ok(ProbeResult {
            ok: true,
            status: Some(status),
            latency_ms: ms,
            ..ProbeResult::default()
        })
    };
    let mut result = finish(Some(&url), run());
    result.model = Some(model);
    result
}

fn efforts_with(client: &Client, target: &Target, model: Option<&str>) -> ProbeResult {
    let levels: &[&str] = match target.protocol {
        Protocol::Anthropic | Protocol::AnthropicV1 => ANTHROPIC_EFFORTS,
        Protocol::OpenAiChat | Protocol::OpenAiResponses => OPENAI_EFFORTS,
        Protocol::Gemini => return finish(None, Err(Failure::detail("unsupported", "effort"))),
    };
    let Some(model) = chosen_model(target, model) else {
        return finish(None, Err(Failure::new("no_model")));
    };
    let url = check_url(target, &model);
    let run = || -> Result<ProbeResult, Failure> {
        let (status, bytes, mut total) = minimal(client, target, &url, &model, None)?;
        accepted(status, &bytes, target)?;
        let mut results = Vec::new();
        for level in levels {
            let (status, bytes, ms) = minimal(client, target, &url, &model, Some(level))?;
            total += ms;
            let supported = match accepted(status, &bytes, target) {
                Ok(()) => true,
                Err(failure) if matches!(failure.code, "rejected") => {
                    results.push(LevelResult {
                        level: level.to_string(),
                        supported: false,
                        detail: failure.detail,
                    });
                    continue;
                }
                Err(failure) => return Err(failure),
            };
            results.push(LevelResult {
                level: level.to_string(),
                supported,
                detail: None,
            });
        }
        Ok(ProbeResult {
            ok: true,
            status: Some(200),
            latency_ms: total,
            levels: Some(results),
            ..ProbeResult::default()
        })
    };
    let mut result = finish(Some(&url), run());
    result.model = Some(model);
    result
}

fn check_body(protocol: Protocol, model: &str, limit: &str, effort: Option<&str>) -> Value {
    let messages = json!([{ "role": "user", "content": "ping" }]);
    let mut body = match protocol {
        Protocol::Anthropic | Protocol::AnthropicV1 => {
            json!({ "model": model, "max_tokens": CHECK_TOKENS, "messages": messages })
        }
        Protocol::OpenAiChat => {
            json!({ "model": model, limit: CHECK_TOKENS, "messages": messages })
        }
        Protocol::OpenAiResponses => {
            json!({ "model": model, "input": "ping", "max_output_tokens": CHECK_TOKENS, "store": false })
        }
        Protocol::Gemini => json!({
            "contents": [{ "role": "user", "parts": [{ "text": "ping" }] }],
            "generationConfig": { "maxOutputTokens": CHECK_TOKENS }
        }),
    };
    if let Some(effort) = effort {
        match protocol {
            Protocol::Anthropic | Protocol::AnthropicV1 => {
                body["output_config"] = json!({ "effort": effort })
            }
            Protocol::OpenAiResponses => body["reasoning"] = json!({ "effort": effort }),
            Protocol::OpenAiChat => body["reasoning_effort"] = json!(effort),
            Protocol::Gemini => {}
        }
    }
    body
}

fn join(base: &Url, segments: &[&str]) -> Url {
    let mut url = base.clone();
    if let Ok(mut path) = url.path_segments_mut() {
        path.pop_if_empty();
        path.extend(segments);
    }
    url
}

fn models_url(target: &Target) -> Url {
    match target.protocol {
        Protocol::Anthropic => join(&target.base, &["v1", "models"]),
        Protocol::Gemini => join(&target.base, &["v1beta", "models"]),
        _ => join(&target.base, &["models"]),
    }
}

fn check_url(target: &Target, model: &str) -> Url {
    match target.protocol {
        Protocol::Anthropic => join(&target.base, &["v1", "messages"]),
        Protocol::AnthropicV1 => join(&target.base, &["messages"]),
        Protocol::OpenAiChat => join(&target.base, &["chat", "completions"]),
        Protocol::OpenAiResponses => join(&target.base, &["responses"]),
        Protocol::Gemini => {
            let model = model.strip_prefix("models/").unwrap_or(model);
            join(
                &target.base,
                &["v1beta", "models", &format!("{model}:generateContent")],
            )
        }
    }
}

fn authorize(target: &Target, request: RequestBuilder) -> RequestBuilder {
    let mut request = match &target.auth {
        Auth::None => request,
        Auth::Bearer(key) => request.bearer_auth(key),
        Auth::ApiKey(key) => request.header("x-api-key", key),
        Auth::Google(key) => request.header("x-goog-api-key", key),
    };
    if matches!(target.protocol, Protocol::Anthropic | Protocol::AnthropicV1) {
        request = request.header("anthropic-version", ANTHROPIC_VERSION);
    }
    for (name, value) in &target.headers {
        request = request.header(name, value);
    }
    if !target.query.is_empty() {
        request = request.query(&target.query);
    }
    request
}

fn send(request: RequestBuilder) -> Result<(Response, u64), Failure> {
    let started = Instant::now();
    let response = request.send().map_err(network_failure)?;
    Ok((response, started.elapsed().as_millis() as u64))
}

fn read(response: Response) -> Result<(u16, Vec<u8>), Failure> {
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .take(MAX_BODY)
        .read_to_end(&mut body)
        .map_err(|_| Failure::new("network"))?;
    Ok((status, body))
}

fn network_failure(error: reqwest::Error) -> Failure {
    if error.is_timeout() {
        return Failure::new("timeout");
    }
    if error.is_builder() {
        return Failure::new("invalid_config");
    }
    let mut chain = String::new();
    let mut source: Option<&dyn std::error::Error> = Some(&error);
    while let Some(current) = source {
        chain.push_str(&current.to_string().to_ascii_lowercase());
        chain.push(' ');
        source = current.source();
    }
    if chain.contains("certificate") || chain.contains("tls") || chain.contains("handshake") {
        Failure::new("tls")
    } else if chain.contains("dns") || chain.contains("lookup") || chain.contains("resolve") {
        Failure::new("dns")
    } else if error.is_connect() {
        Failure::new("connect")
    } else {
        Failure::new("network")
    }
}

fn http_failure(status: u16, body: &[u8], secrets: &[&str]) -> Failure {
    let code = match status {
        300..=399 => "redirect",
        400 | 422 => "rejected",
        401 | 403 => "auth",
        404 | 405 => "not_found",
        408 | 504 => "timeout",
        429 => "rate_limited",
        500..=599 => "server",
        _ => "http",
    };
    Failure {
        code,
        detail: message(body, secrets),
        status: Some(status),
    }
}

/// 供应商错误说明：取 JSON 里的 message，或简短的纯文本；遮掉密钥，截断到 300 字。
fn message(body: &[u8], secrets: &[&str]) -> Option<String> {
    let text = match serde_json::from_slice::<Value>(body) {
        Ok(value) => ["/error/message", "/message", "/error", "/detail", "/msg"]
            .iter()
            .find_map(|path| value.pointer(path).and_then(Value::as_str))
            .map(String::from)?,
        Err(_) => {
            let text = String::from_utf8_lossy(body).to_string();
            if text.trim_start().starts_with('<') || text.len() > 2000 {
                return None;
            }
            text
        }
    };
    let mut text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for secret in secrets {
        text = text.replace(secret, "***");
    }
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > 300 {
        text = text.chars().take(300).collect::<String>() + "…";
    }
    Some(text)
}

fn display(url: &Url) -> String {
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn parse_models(protocol: Protocol, value: &Value) -> Option<(Vec<RemoteModel>, Option<String>)> {
    let string = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    match protocol {
        Protocol::Gemini => {
            let models = value.get("models")?.as_array()?;
            let list = models
                .iter()
                .filter(|m| {
                    m.get("supportedGenerationMethods")
                        .and_then(Value::as_array)
                        .is_none_or(|methods| {
                            methods
                                .iter()
                                .any(|x| x.as_str() == Some("generateContent"))
                        })
                })
                .filter_map(|m| {
                    let id = string(m, "name")?;
                    Some(RemoteModel {
                        id: id.strip_prefix("models/").unwrap_or(&id).to_string(),
                        name: string(m, "displayName"),
                        efforts: None,
                    })
                })
                .collect();
            Some((list, string(value, "nextPageToken")))
        }
        _ => {
            let items = value
                .get("data")
                .or_else(|| value.get("models"))
                .unwrap_or(value)
                .as_array()?;
            let list = items
                .iter()
                .filter_map(|m| match m {
                    Value::String(id) if !id.trim().is_empty() => Some(RemoteModel {
                        id: id.trim().to_string(),
                        ..RemoteModel::default()
                    }),
                    _ => Some(RemoteModel {
                        id: string(m, "id").or_else(|| string(m, "name"))?,
                        name: string(m, "display_name")
                            .or_else(|| string(m, "displayName"))
                            .filter(|_| m.get("id").is_some()),
                        efforts: effort_levels(m),
                    }),
                })
                .collect();
            let next = (value.get("has_more").and_then(Value::as_bool) == Some(true))
                .then(|| string(value, "last_id"))
                .flatten();
            Some((list, next))
        }
    }
}

/// Anthropic 模型列表的 `capabilities.effort`：逐档标明是否支持；不支持 effort 时为空列表。
fn effort_levels(model: &Value) -> Option<Vec<String>> {
    let effort = model.get("capabilities")?.get("effort")?;
    if effort.get("supported").and_then(Value::as_bool) == Some(false) {
        return Some(Vec::new());
    }
    Some(
        ANTHROPIC_EFFORTS
            .iter()
            .filter(|level| {
                effort
                    .get(**level)
                    .and_then(|l| l.get("supported"))
                    .and_then(Value::as_bool)
                    == Some(true)
            })
            .map(|level| level.to_string())
            .collect(),
    )
}

// ---- 从供应商配置里取出地址、凭据和模型。

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut node = value;
    for part in path {
        node = node.get(part)?;
    }
    node.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn from_env(name: &str) -> Result<String, Failure> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| Failure::detail("key_env", name))
}

/// `{env:NAME}`（OpenCode）或 `${NAME}` 形式的密钥从当前进程环境读取。
fn secret(raw: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(raw) = raw else { return Ok(None) };
    let reference = raw
        .strip_prefix("{env:")
        .or_else(|| raw.strip_prefix("${"))
        .and_then(|rest| rest.strip_suffix('}'));
    match reference {
        Some(name) => from_env(name.trim()).map(Some),
        None => Ok(Some(raw.to_string())),
    }
}

fn base(raw: Option<&str>, default: Option<&str>) -> Result<Url, Failure> {
    let raw = raw.or(default).ok_or_else(|| Failure::new("no_base_url"))?;
    let mut url = Url::parse(raw).map_err(|_| Failure::new("invalid_url"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Failure::new("invalid_url"));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn flag(config: &Value, key: &str) -> bool {
    config
        .get("env")
        .and_then(|env| env.get(key))
        .is_some_and(|v| {
            v == &Value::Bool(true) || v.as_str().is_some_and(|s| matches!(s, "1" | "true"))
        })
}

fn api_protocol(api: Option<&str>) -> Protocol {
    match api.unwrap_or_default() {
        api if api.contains("anthropic") => Protocol::Anthropic,
        api if api.contains("responses") => Protocol::OpenAiResponses,
        _ => Protocol::OpenAiChat,
    }
}

fn first_model(config: &Value) -> Option<String> {
    match config.get("models")? {
        Value::Object(models) => models.keys().next().cloned(),
        Value::Array(models) => models.iter().find_map(|m| match m {
            Value::String(id) => Some(id.clone()),
            _ => text(m, &["id"])
                .or_else(|| text(m, &["name"]))
                .map(String::from),
        }),
        _ => None,
    }
}

fn table_strings(item: Option<&Item>) -> Vec<(String, String)> {
    item.and_then(Item::as_table_like)
        .map(|table| {
            table
                .iter()
                .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn document(config: &Value) -> Result<DocumentMut, Failure> {
    config
        .get("config")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| Failure::new("invalid_config"))
}

fn target(tool: Tool, config: &Value) -> Result<Target, Failure> {
    let plain = |protocol, base, auth, model| Target {
        protocol,
        base,
        auth,
        model,
        headers: Vec::new(),
        query: Vec::new(),
    };
    let bearer = |key: Option<String>| key.map_or(Auth::None, Auth::Bearer);
    match tool {
        Tool::Claude | Tool::ClaudeDesktop => {
            let env = |key: &str| text(config, &["env", key]);
            if [
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
            ]
            .iter()
            .any(|key| flag(config, key))
            {
                return Err(Failure::detail("unsupported", "advanced_auth"));
            }
            let auth = match secret(env("ANTHROPIC_AUTH_TOKEN"))? {
                Some(token) => Auth::Bearer(token),
                None if tool == Tool::Claude => {
                    secret(env("ANTHROPIC_API_KEY"))?.map_or(Auth::None, Auth::ApiKey)
                }
                None => Auth::None,
            };
            let model = [
                "ANTHROPIC_MODEL",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
            ]
            .iter()
            .find_map(|key| env(key))
            .map(String::from);
            Ok(plain(
                Protocol::Anthropic,
                base(env("ANTHROPIC_BASE_URL"), Some("https://api.anthropic.com"))?,
                auth,
                model,
            ))
        }
        Tool::Gemini => {
            let env = |key: &str| text(config, &["env", key]);
            if flag(config, "GOOGLE_GENAI_USE_VERTEXAI") {
                return Err(Failure::detail("unsupported", "advanced_auth"));
            }
            let key = secret(env("GEMINI_API_KEY").or_else(|| env("GOOGLE_API_KEY")))?;
            let model = env("GEMINI_MODEL")
                .or_else(|| text(config, &["config", "model", "name"]))
                .map(String::from);
            Ok(plain(
                Protocol::Gemini,
                base(
                    env("GOOGLE_GEMINI_BASE_URL"),
                    Some("https://generativelanguage.googleapis.com"),
                )?,
                key.map_or(Auth::None, Auth::Google),
                model,
            ))
        }
        Tool::Codex => {
            let doc = document(config)?;
            let model = doc.get("model").and_then(Item::as_str).map(String::from);
            let stored_key = text(config, &["auth", "OPENAI_API_KEY"])
                .or_else(|| text(config, &["OPENAI_API_KEY"]))
                .map(String::from);
            let tables = doc.get("model_providers").and_then(Item::as_table_like);
            // Like Codex: without model_provider the built-in openai provider is used, even
            // when a [model_providers] table is left over from another provider.
            let id = doc
                .get("model_provider")
                .and_then(Item::as_str)
                .unwrap_or("openai")
                .to_string();
            match id.as_str() {
                "openai" => Ok(plain(
                    Protocol::OpenAiResponses,
                    base(
                        doc.get("openai_base_url").and_then(Item::as_str),
                        Some("https://api.openai.com/v1"),
                    )?,
                    bearer(stored_key),
                    model,
                )),
                "ollama" | "lmstudio" => Err(Failure::detail("unsupported", "builtin_local")),
                _ => {
                    let table = tables
                        .and_then(|t| t.get(&id))
                        .and_then(Item::as_table_like)
                        .ok_or_else(|| {
                            Failure::detail("invalid_config", format!("model_providers.{id}"))
                        })?;
                    let field = |key: &str| {
                        table
                            .get(key)
                            .and_then(Item::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    };
                    let key = match (field("experimental_bearer_token"), field("env_key")) {
                        (Some(token), _) => Some(token.to_string()),
                        (None, Some(name)) => Some(from_env(name)?),
                        (None, None) => stored_key,
                    };
                    Ok(Target {
                        protocol: if field("wire_api") == Some("chat") {
                            Protocol::OpenAiChat
                        } else {
                            Protocol::OpenAiResponses
                        },
                        base: base(field("base_url"), None)?,
                        auth: bearer(key),
                        model,
                        headers: table_strings(table.get("http_headers")),
                        query: table_strings(table.get("query_params")),
                    })
                }
            }
        }
        Tool::GrokBuild => {
            let doc = document(config)?;
            let tables = doc.get("model").and_then(Item::as_table_like);
            let name = match doc
                .get("models")
                .and_then(|m| m.get("default"))
                .and_then(Item::as_str)
            {
                Some(name) => name.to_string(),
                None => match tables {
                    Some(tables) if tables.len() == 1 => {
                        tables.iter().next().unwrap().0.to_string()
                    }
                    _ => return Err(Failure::detail("invalid_config", "models.default")),
                },
            };
            let table = tables
                .and_then(|t| t.get(&name))
                .and_then(Item::as_table_like)
                .ok_or_else(|| Failure::detail("invalid_config", format!("model.{name}")))?;
            let field = |key: &str| {
                table
                    .get(key)
                    .and_then(Item::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            };
            let key = match (field("api_key"), field("env_key")) {
                (Some(key), _) => Some(key.to_string()),
                (None, Some(name)) => Some(from_env(name)?),
                (None, None) => None,
            };
            Ok(plain(
                if field("api_backend").is_some_and(|b| b.contains("chat")) {
                    Protocol::OpenAiChat
                } else {
                    Protocol::OpenAiResponses
                },
                base(field("base_url"), None)?,
                bearer(key),
                field("model").map(String::from).or(Some(name)),
            ))
        }
        Tool::OpenCode | Tool::Mcode => {
            let protocol = if tool == Tool::OpenCode {
                match text(config, &["npm"]) {
                    Some("@ai-sdk/anthropic") => Protocol::AnthropicV1,
                    _ => Protocol::OpenAiChat,
                }
            } else {
                api_protocol(text(config, &["api"]))
            };
            let key = secret(text(config, &["options", "apiKey"]))?;
            let auth = match (protocol, key) {
                (_, None) => Auth::None,
                (Protocol::AnthropicV1 | Protocol::Anthropic, Some(key)) => Auth::ApiKey(key),
                (_, Some(key)) => Auth::Bearer(key),
            };
            Ok(plain(
                protocol,
                base(
                    text(config, &["options", "baseURL"]).or_else(|| text(config, &["baseURL"])),
                    None,
                )?,
                auth,
                first_model(config),
            ))
        }
        Tool::OpenClaw | Tool::Pi | Tool::Hermes => {
            let pick = |keys: &[&str]| keys.iter().find_map(|key| text(config, &[key]));
            let protocol = api_protocol(pick(&["api", "api_mode", "apiMode"]));
            let key = secret(pick(&["apiKey", "api_key"]))?;
            let auth = match (protocol, key) {
                (_, None) => Auth::None,
                (Protocol::Anthropic, Some(key)) => Auth::ApiKey(key),
                (_, Some(key)) => Auth::Bearer(key),
            };
            Ok(plain(
                protocol,
                base(pick(&["baseUrl", "base_url", "baseURL"]), None)?,
                auth,
                first_model(config),
            ))
        }
    }
}

#[cfg(test)]
mod tests;
