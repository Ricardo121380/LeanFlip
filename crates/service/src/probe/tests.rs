use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One canned response per connection; the client gets `Connection: close` each time.
fn serve(responses: Vec<(u16, &'static str, String)>) -> (Url, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for (status, extra, body) in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let mut headers = Vec::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some((k, v)) = header.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let length = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.parse::<usize>().ok())
                .unwrap_or(0);
            let mut content = vec![0; length];
            reader.read_exact(&mut content).unwrap();
            log.lock().unwrap().push(Seen {
                path,
                headers,
                body: String::from_utf8_lossy(&content).into(),
            });
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (Url::parse(&format!("http://{address}")).unwrap(), seen)
}

fn local() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn at(base: &Url, path: &str, protocol: Protocol, auth: Auth) -> Target {
    Target {
        protocol,
        base: base.join(path).unwrap(),
        auth,
        model: Some("m1".into()),
        headers: Vec::new(),
        query: Vec::new(),
    }
}

#[test]
fn latency_counts_any_http_answer_and_sends_no_credentials() {
    let (base, seen) = serve(vec![(401, "", "{}".into())]);
    let target = at(
        &base,
        "/v1",
        Protocol::OpenAiChat,
        Auth::Bearer("sk-fixture-secret".into()),
    );
    let result = latency_with(&local(), &target);
    assert!(result.ok);
    assert_eq!(result.status, Some(401));
    assert_eq!(
        result.endpoint.as_deref(),
        Some(format!("{base}v1").as_str())
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1");
    assert!(seen[0].header("authorization").is_none());
}

#[test]
fn anthropic_models_follow_pages_with_version_and_key_headers() {
    let (base, seen) = serve(vec![
        (
            200,
            "",
            json!({"data":[{"id":"a","display_name":"Model A","capabilities":{"effort":{"supported":true,"low":{"supported":true},"medium":{"supported":true},"high":{"supported":true},"xhigh":null,"max":{"supported":false}}}}],"has_more":true,"last_id":"a"})
                .to_string(),
        ),
        (
            200,
            "",
            json!({"data":[{"id":"b"},{"id":"a"}],"has_more":false}).to_string(),
        ),
    ]);
    let target = at(
        &base,
        "/",
        Protocol::Anthropic,
        Auth::ApiKey("sk-fixture-secret".into()),
    );
    let result = models_with(&local(), &target);
    assert!(result.ok, "{result:?}");
    assert_eq!(
        result.models.unwrap(),
        vec![
            RemoteModel {
                id: "a".into(),
                name: Some("Model A".into()),
                efforts: Some(vec!["low".into(), "medium".into(), "high".into()]),
            },
            RemoteModel {
                id: "b".into(),
                ..RemoteModel::default()
            },
        ]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1/models?limit=1000");
    assert_eq!(seen[1].path, "/v1/models?limit=1000&after_id=a");
    assert_eq!(seen[0].header("x-api-key"), Some("sk-fixture-secret"));
    assert_eq!(seen[0].header("anthropic-version"), Some(ANTHROPIC_VERSION));
}

#[test]
fn failures_are_classified_and_never_echo_the_key() {
    let (base, _) = serve(vec![(
        401,
        "",
        json!({"error":{"message":"invalid key sk-fixture-secret\nrotate it"}}).to_string(),
    )]);
    let target = at(
        &base,
        "/v1",
        Protocol::OpenAiResponses,
        Auth::Bearer("sk-fixture-secret".into()),
    );
    let result = check_with(&local(), &target, None);
    assert!(!result.ok);
    assert_eq!(result.code.as_deref(), Some("auth"));
    assert_eq!(result.status, Some(401));
    assert_eq!(result.detail.as_deref(), Some("invalid key *** rotate it"));
    assert_eq!(result.model.as_deref(), Some("m1"));
}

#[test]
fn chat_check_retries_once_with_max_completion_tokens() {
    let (base, seen) = serve(vec![
        (400, "", json!({"error":{"message":"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."}}).to_string()),
        (200, "", json!({"choices":[]}).to_string()),
    ]);
    let target = at(
        &base,
        "/v1",
        Protocol::OpenAiChat,
        Auth::Bearer("sk-fixture-secret".into()),
    );
    let result = check_with(&local(), &target, Some("gpt-x"));
    assert!(result.ok, "{result:?}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1/chat/completions");
    assert!(seen[0].body.contains("\"max_tokens\":16"));
    assert!(seen[1].body.contains("\"max_completion_tokens\":16"));
    assert!(seen[1].body.contains("\"model\":\"gpt-x\""));
}

#[test]
fn redirects_are_reported_instead_of_followed() {
    let (base, seen) = serve(vec![(
        302,
        "Location: http://elsewhere.invalid/models\r\n",
        String::new(),
    )]);
    let target = at(
        &base,
        "/v1",
        Protocol::OpenAiChat,
        Auth::Bearer("sk-fixture-secret".into()),
    );
    let result = models_with(&local(), &target);
    assert_eq!(result.code.as_deref(), Some("redirect"));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn gemini_uses_the_header_key_and_model_path() {
    let (base, seen) = serve(vec![(200, "", json!({"candidates":[]}).to_string())]);
    let target = at(
        &base,
        "/",
        Protocol::Gemini,
        Auth::Google("fixture-google-key".into()),
    );
    assert!(check_with(&local(), &target, Some("models/gemini-x")).ok);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1beta/models/gemini-x:generateContent");
    assert_eq!(seen[0].header("x-goog-api-key"), Some("fixture-google-key"));
    assert!(!seen[0].path.contains("key="));
}

#[test]
fn gemini_models_keep_only_generation_models() {
    let value = json!({"models":[
        {"name":"models/gemini-x","displayName":"Gemini X","supportedGenerationMethods":["generateContent"]},
        {"name":"models/embedding","supportedGenerationMethods":["embedContent"]}
    ],"nextPageToken":"p2"});
    let (models, next) = parse_models(Protocol::Gemini, &value).unwrap();
    assert_eq!(
        models,
        vec![RemoteModel {
            id: "gemini-x".into(),
            name: Some("Gemini X".into()),
            efforts: None,
        }]
    );
    assert_eq!(next.as_deref(), Some("p2"));
}

fn code(result: Result<Target, Failure>) -> (&'static str, Option<String>) {
    let failure = result.unwrap_err();
    (failure.code, failure.detail)
}

#[test]
fn claude_targets_follow_the_client_auth_rules() {
    let token = target(Tool::Claude, &json!({"env":{"ANTHROPIC_BASE_URL":"https://gw.example/anthropic","ANTHROPIC_AUTH_TOKEN":"t","ANTHROPIC_DEFAULT_SONNET_MODEL":"s"}})).unwrap();
    assert_eq!(token.auth, Auth::Bearer("t".into()));
    assert_eq!(
        check_url(&token, "s").as_str(),
        "https://gw.example/anthropic/v1/messages"
    );
    assert_eq!(token.model.as_deref(), Some("s"));
    let key = target(Tool::Claude, &json!({"env":{"ANTHROPIC_API_KEY":"k"}})).unwrap();
    assert_eq!(key.auth, Auth::ApiKey("k".into()));
    assert_eq!(key.base.as_str(), "https://api.anthropic.com/");
    let desktop = target(
        Tool::ClaudeDesktop,
        &json!({"env":{"ANTHROPIC_BASE_URL":"https://gw.example","ANTHROPIC_API_KEY":"k"}}),
    )
    .unwrap();
    assert_eq!(desktop.auth, Auth::None);
    assert_eq!(
        code(target(
            Tool::Claude,
            &json!({"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}})
        )),
        ("unsupported", Some("advanced_auth".into()))
    );
}

#[test]
fn codex_targets_cover_builtin_and_custom_providers() {
    let builtin = target(Tool::Codex, &json!({"auth":{"OPENAI_API_KEY":"k"},"config":"model = \"m\"\nopenai_base_url = \"https://proxy.example/v1\"\n"})).unwrap();
    assert_eq!(builtin.base.as_str(), "https://proxy.example/v1");
    assert_eq!(builtin.protocol, Protocol::OpenAiResponses);
    assert_eq!(
        check_url(&builtin, "m").as_str(),
        "https://proxy.example/v1/responses"
    );

    let custom = target(Tool::Codex, &json!({"auth":{"OPENAI_API_KEY":"k"},"config":"model_provider = \"az\"\n[model_providers.az]\nbase_url = \"https://az.example/openai\"\nquery_params = { api-version = \"2025-04-01\" }\nhttp_headers = { X-Team = \"a\" }\n"})).unwrap();
    assert_eq!(custom.auth, Auth::Bearer("k".into()));
    assert_eq!(
        custom.query,
        vec![("api-version".into(), "2025-04-01".into())]
    );
    assert_eq!(custom.headers, vec![("X-Team".into(), "a".into())]);

    assert_eq!(
        code(target(
            Tool::Codex,
            &json!({"config":"model_provider = \"ollama\"\n"})
        )),
        ("unsupported", Some("builtin_local".into()))
    );
    assert_eq!(
        code(target(
            Tool::Codex,
            &json!({"config":"model_provider = \"x\"\n[model_providers.x]\nbase_url = \"https://x.example\"\nenv_key = \"LEANFLIP_TEST_UNSET_KEY\"\n"})
        )),
        ("key_env", Some("LEANFLIP_TEST_UNSET_KEY".into()))
    );
}

#[test]
fn grok_and_coexist_targets_map_their_protocols() {
    let grok = target(Tool::GrokBuild, &json!({"config":"[models]\ndefault = \"c\"\n[model.c]\nmodel = \"grok-x\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"xk\"\napi_backend = \"responses\"\n"})).unwrap();
    assert_eq!(
        (grok.protocol, grok.model.as_deref()),
        (Protocol::OpenAiResponses, Some("grok-x"))
    );

    let opencode = target(Tool::OpenCode, &json!({"npm":"@ai-sdk/anthropic","options":{"baseURL":"https://api.example.com/v1","apiKey":"k"},"models":{"claude-x":{}}})).unwrap();
    assert_eq!(opencode.protocol, Protocol::AnthropicV1);
    assert_eq!(opencode.auth, Auth::ApiKey("k".into()));
    assert_eq!(
        check_url(&opencode, "claude-x").as_str(),
        "https://api.example.com/v1/messages"
    );

    let pi = target(Tool::Pi, &json!({"baseUrl":"https://api.example.com","api":"anthropic-messages","apiKey":"k","models":[{"id":"p1"}]})).unwrap();
    assert_eq!(
        (pi.protocol, pi.model.as_deref()),
        (Protocol::Anthropic, Some("p1"))
    );

    let hermes = target(
        Tool::Hermes,
        &json!({"base_url":"https://api.example.com/v1","api_key":"k","models":[{"id":"h1"}]}),
    )
    .unwrap();
    assert_eq!(hermes.protocol, Protocol::OpenAiChat);

    assert_eq!(
        code(target(Tool::OpenClaw, &json!({"apiKey":"k"}))).0,
        "no_base_url"
    );
    assert_eq!(
        code(target(Tool::OpenClaw, &json!({"baseUrl":"ftp://x"}))).0,
        "invalid_url"
    );
    assert_eq!(
        code(target(
            Tool::OpenCode,
            &json!({"options":{"baseURL":"https://x.example","apiKey":"{env:LEANFLIP_TEST_UNSET_KEY}"}})
        )),
        ("key_env", Some("LEANFLIP_TEST_UNSET_KEY".into()))
    );
}

#[test]
fn effort_levels_are_probed_after_a_baseline_and_rejections_mark_unsupported() {
    let ok = || (200, "", json!({"output":[]}).to_string());
    let rejected = || {
        (400, "", json!({"error":{"message":"Unsupported value: 'reasoning.effort' does not support this value"}}).to_string())
    };
    let (base, seen) = serve(vec![
        ok(),
        rejected(),
        ok(),
        ok(),
        ok(),
        rejected(),
        rejected(),
    ]);
    let target = at(
        &base,
        "/v1",
        Protocol::OpenAiResponses,
        Auth::Bearer("sk-fixture-secret".into()),
    );
    let result = efforts_with(&local(), &target, Some("gpt-x"));
    assert!(result.ok, "{result:?}");
    let levels = result.levels.unwrap();
    let supported: Vec<_> = levels
        .iter()
        .filter(|l| l.supported)
        .map(|l| l.level.as_str())
        .collect();
    assert_eq!(supported, ["low", "medium", "high"]);
    assert!(levels[0]
        .detail
        .as_deref()
        .unwrap()
        .contains("Unsupported value"));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 7);
    assert!(!seen[0].body.contains("reasoning"));
    assert!(seen[1]
        .body
        .contains("\"reasoning\":{\"effort\":\"minimal\"}"));
}

#[test]
fn effort_probe_stops_when_the_baseline_fails() {
    let (base, seen) = serve(vec![(
        401,
        "",
        json!({"error":{"message":"bad key"}}).to_string(),
    )]);
    let target = at(
        &base,
        "/",
        Protocol::Anthropic,
        Auth::ApiKey("sk-fixture-secret".into()),
    );
    let result = efforts_with(&local(), &target, None);
    assert_eq!(result.code.as_deref(), Some("auth"));
    assert!(result.levels.is_none());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn effort_parameters_follow_each_protocol() {
    let body = check_body(Protocol::Anthropic, "m", "max_tokens", Some("xhigh"));
    assert_eq!(body["output_config"], json!({"effort":"xhigh"}));
    let chat = check_body(Protocol::OpenAiChat, "m", "max_tokens", Some("low"));
    assert_eq!(chat["reasoning_effort"], json!("low"));
    assert!(
        check_body(Protocol::OpenAiResponses, "m", "max_tokens", None)
            .get("reasoning")
            .is_none()
    );
}
