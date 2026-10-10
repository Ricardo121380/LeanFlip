//! 开发用：在一个**临时** HOME 里造演示数据（假的客户端配置、几张卡、两个假托管账号），
//! 给无头冒烟运行 / 截图用。拒绝在系统临时目录以外的地方运行，永远不碰真实 HOME。
//!
//!     cargo run -p ccsl-app --example seed_demo -- /tmp/ccsl-demo-XXXX

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ccsl_app::service::{PollResult, Service};
use ccsl_core::codex::oauth::testing::{FakeTransport, Identity};
use ccsl_core::{Env, Provider, Tool};
use serde_json::json;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn main() {
    let home = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: seed_demo <temp home>"),
    );
    let temp = std::env::temp_dir().canonicalize().unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let home = home.canonicalize().unwrap();
    assert!(
        home.starts_with(&temp) && home != temp,
        "refusing to seed outside {}: {}",
        temp.display(),
        home.display()
    );
    let env = Env::with_home(&home);
    for tool in Tool::ALL {
        for path in env.live_paths(tool) {
            assert!(path.starts_with(&home));
        }
    }

    // 客户端现有配置。
    write(
        &env.claude_settings_path(),
        r#"{
  "env": { "ANTHROPIC_BASE_URL": "https://api.example-relay.dev", "ANTHROPIC_AUTH_TOKEN": "sk-demo-relay" },
  "permissions": { "allow": ["Bash(ls:*)"] }
}
"#,
    );
    write(
        &env.codex_config_path(),
        "# my codex config\nmodel = \"gpt-5.5\"\nmodel_reasoning_effort = \"high\"\n",
    );
    write(&env.gemini_env_path(), "GEMINI_API_KEY=demo-gemini-key\n");
    write(
        &env.opencode_dir.join("opencode.json"),
        r#"{
  "$schema": "https://opencode.ai/config.json",
  "provider": {
    "local-relay": { "npm": "@ai-sdk/openai-compatible", "name": "Local relay", "options": { "baseURL": "https://relay.example/v1", "apiKey": "sk-demo" }, "models": { "gpt-5": {} } }
  }
}
"#,
    );

    let fake = Arc::new(FakeTransport::default());
    let service = Service::start(env.clone(), Some(fake.clone())).unwrap();

    // 两个假托管账号（Plus / Team）。
    let mut team_id = String::new();
    for (user, ws, plan) in [
        ("alice", "ws-personal", "plus"),
        ("alice-work", "ws-acme", "team"),
    ] {
        let flow = service.codex_start_login(None).unwrap();
        fake.approve(&flow.device_code, &Identity::new(user, ws, plan));
        match service.codex_poll_login(&flow.device_code).unwrap() {
            PollResult::Added { account } => {
                if plan == "team" {
                    team_id = account.id;
                }
            }
            PollResult::Pending => panic!("fake login should finish"),
        }
    }

    // Claude Code：导入现有配置 + 两张卡，切到 Kimi。
    service.import_live(Tool::Claude).unwrap();
    let kimi = Provider::new(
        "kimi",
        "Kimi",
        json!({"env": {"ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic", "ANTHROPIC_AUTH_TOKEN": "sk-demo-kimi", "ANTHROPIC_MODEL": "kimi-k2.7-code"}}),
    );
    let deepseek = Provider::new(
        "deepseek",
        "DeepSeek",
        json!({"env": {"ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic", "ANTHROPIC_AUTH_TOKEN": "sk-demo-ds"}}),
    );
    service.add_provider(Tool::Claude, kimi, true).unwrap();
    service.add_provider(Tool::Claude, deepseek, false).unwrap();

    // Codex：官方卡绑定 Team 托管账号 + 一张第三方卡。
    service.import_live(Tool::Codex).unwrap();
    let mut work = Provider::new(
        "openai-work",
        "OpenAI (work)",
        json!({"auth": {}, "config": "model = \"gpt-5.5\"\n"}),
    )
    .official();
    work.meta.auth_binding = Some(ccsl_core::provider::AuthBinding {
        auth_provider: ccsl_core::provider::AUTH_PROVIDER_CODEX_OAUTH.into(),
        account_id: Some(team_id),
    });
    service.add_provider(Tool::Codex, work, false).unwrap();
    let relay = Provider::new(
        "packy",
        "PackyCode",
        json!({"auth": {"OPENAI_API_KEY": "sk-demo-packy"},
               "config": "model_provider = \"custom\"\nmodel = \"gpt-5.5\"\n\n[model_providers.custom]\nname = \"PackyCode\"\nbase_url = \"https://www.packyapi.ai/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"}),
    );
    service.add_provider(Tool::Codex, relay, false).unwrap();

    // Gemini：导入。
    service.import_live(Tool::Gemini).unwrap();

    // OpenCode：导入现有条目 + 两张卡，其中一张写进配置。
    service.import_live(Tool::OpenCode).unwrap();
    for (id, name, url, live) in [
        ("kimi", "Kimi", "https://api.moonshot.ai/v1", true),
        ("deepseek", "DeepSeek", "https://api.deepseek.com/v1", false),
    ] {
        let card = Provider::new(
            id,
            name,
            json!({"npm": "@ai-sdk/openai-compatible", "name": name,
                   "options": {"baseURL": url, "apiKey": "sk-demo"}, "models": {"m": {}}}),
        );
        service.add_provider(Tool::OpenCode, card, live).unwrap();
    }

    // 界面语言：中文；再放一份假的 cc-switch 旧版数据给导入向导。
    let mut settings = service.app_settings();
    settings.language = Some(std::env::var("CCSL_DEMO_LANG").unwrap_or_else(|_| "zh".into()));
    ccsl_app::settings::save(&env.data_dir, &settings).unwrap();
    write(
        &home.join(".cc-switch/config.json"),
        &serde_json::to_string_pretty(&json!({
            "claude": {"providers": {
                "zhipu": {"id": "zhipu", "name": "智谱 GLM", "settingsConfig": {"env": {"ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic", "ANTHROPIC_AUTH_TOKEN": "sk-demo"}}},
                "kimi": {"id": "kimi", "name": "Kimi", "settingsConfig": {"env": {"ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic"}}},
                "copilot": {"id": "copilot", "name": "GitHub Copilot", "providerType": "github_copilot", "settingsConfig": {"env": {}}}
            }},
            "gemini": {"providers": {
                "packy": {"id": "packy", "name": "PackyCode", "settingsConfig": {"env": {"GOOGLE_GEMINI_BASE_URL": "https://www.packyapi.ai", "GEMINI_API_KEY": "sk-demo"}}}
            }}
        }))
        .unwrap(),
    );

    println!("seeded demo HOME at {}", home.display());
}
