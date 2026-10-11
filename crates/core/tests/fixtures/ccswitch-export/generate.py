#!/usr/bin/env python3
"""生成 cc-switch 导出文件的测试样本（格式照 cc-switch `database/backup.rs::dump_sql`）。
所有 key / token 都是假的。重新生成：python3 generate.py"""
import json, os

HERE = os.path.dirname(os.path.abspath(__file__))

def q(v):
    if v is None:
        return "NULL"
    if isinstance(v, bool):
        return "1" if v else "0"
    if isinstance(v, int):
        return str(v)
    if not isinstance(v, str):
        v = json.dumps(v, ensure_ascii=False, separators=(",", ":"))
    return "'" + v.replace("'", "''") + "'"

PROVIDERS_V4 = """CREATE TABLE providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                website_url TEXT,
                category TEXT,
                created_at INTEGER,
                sort_index INTEGER,
                notes TEXT,
                icon TEXT,
                icon_color TEXT,
                meta TEXT NOT NULL DEFAULT '{}',
                is_current BOOLEAN NOT NULL DEFAULT 0,
                in_failover_queue BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            )"""
OTHER_TABLES = [
    """CREATE TABLE provider_endpoints (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                url TEXT NOT NULL,
                added_at INTEGER,
                FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
            )""",
    "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT)",
    """CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, server_config TEXT NOT NULL,
            description TEXT, homepage TEXT, docs TEXT, tags TEXT NOT NULL DEFAULT '[]',
            enabled_claude BOOLEAN NOT NULL DEFAULT 0, enabled_codex BOOLEAN NOT NULL DEFAULT 0
        )""",
]

CODEX_RESPONSES = 'model_provider = "custom"\nmodel = "gpt-5.5"\n\n[model_providers.custom]\nname = "relay"\nbase_url = "https://relay.example.com/v1"\nwire_api = "responses"\nrequires_openai_auth = true\n'
# cc-switch 库里存的是 Grok Build 自己的格式（保存时由 grokBuildConfig.ts 生成）。
GROK = '[models]\ndefault = "grok-4.5"\n\n[model."grok-4.5"]\nmodel = "grok-4.5"\nbase_url = "https://grok-relay.example.com/v1"\nname = "relay"\napi_key = "sk-test-grok"\napi_backend = "responses"\ncontext_window = 500000\n'

# (app_type, id, name, settings, website, category, meta, is_current)
ROWS = [
    ("claude", "relay", "Relay", {"env": {"ANTHROPIC_BASE_URL": "https://relay.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-relay"}}, "https://relay.example.com", "third_party", {}, 1),
    ("claude", "kimi", "Kimi", {"env": {"ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic", "ANTHROPIC_AUTH_TOKEN": "sk-test-kimi"}}, "https://platform.kimi.com?track_id=t-1&aff=cc-switch", "cn_official",
     {"isPartner": True, "partnerPromotionKey": "kimi", "usageScript": {"enabled": True, "code": "return 1", "accessToken": "usage-secret"}}, 0),
    ("claude", "openai-chat", "OpenAI via proxy", {"env": {"ANTHROPIC_BASE_URL": "https://o.example.com"}}, None, "custom", {"apiFormat": "openai_chat"}, 0),
    ("claude", "taken", "Taken id", {"env": {"ANTHROPIC_BASE_URL": "https://another.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-other"}}, None, "custom", {}, 0),
    ("claude", "same-as-mine", "Copy of mine", {"env": {"ANTHROPIC_BASE_URL": "https://mine.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-mine"}}, None, "custom", {}, 0),
    ("claude", "claude-official", "Claude Official", {"env": {"CLAUDE_CODE_OAUTH_TOKEN": "oauth-secret"}}, "https://www.anthropic.com/claude-code", "official", {}, 0),
    ("claude-desktop", "desk", "Desktop relay", {"env": {"ANTHROPIC_BASE_URL": "https://desk.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-desk"}}, None, "third_party",
     {"claudeDesktopModelRoutes": {"claude-sonnet-4-5": {"model": "", "labelOverride": "Sonnet"}}}, 1),
    ("claude-desktop", "desk-proxy", "Desktop proxy", {"env": {"ANTHROPIC_BASE_URL": "https://p.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-p"}}, None, "third_party",
     {"claudeDesktopMode": "proxy"}, 0),
    ("codex", "official", "OpenAI Official", {"auth": {"auth_mode": "chatgpt", "OPENAI_API_KEY": None, "tokens": {"id_token": "id-secret", "access_token": "at-secret", "refresh_token": "rt-secret", "account_id": "acct-1"}, "last_refresh": "2026-01-01T00:00:00Z"}, "config": ""}, "https://chatgpt.com/codex", "official",
     {"authBinding": {"authProvider": "codex_oauth", "accountId": "acct-1"}}, 1),
    ("codex", "codex-relay", "Codex relay", {"auth": {"OPENAI_API_KEY": "sk-test-codex"}, "config": CODEX_RESPONSES}, "https://relay.example.com/r/ABC123", "third_party", {}, 0),
    ("codex", "copilot", "GitHub Copilot", {"auth": {}, "config": ""}, None, "third_party", {"providerType": "github_copilot"}, 0),
    ("gemini", "gem", "Gemini relay", {"env": {"GOOGLE_GEMINI_BASE_URL": "https://g.example.com", "GEMINI_API_KEY": "sk-test-gem", "GEMINI_MODEL": "gemini-3.6-flash"}}, None, "third_party", {}, 1),
    ("grokbuild", "grok-relay", "Grok relay", {"config": GROK}, None, "third_party", {}, 0),
    ("grokbuild", "grok-oauth", "Grok (xAI login)", {"auth": {}, "config": ""}, None, "official", {"providerType": "xai_oauth"}, 1),
    ("opencode", "kimi-oc", "Kimi", {"npm": "@ai-sdk/openai-compatible", "name": "Kimi", "options": {"baseURL": "https://api.moonshot.cn/v1", "apiKey": "sk-test-oc"}, "models": {"kimi-k3": {"name": "Kimi K3"}}}, None, "cn_official", {}, 0),
    ("opencode", "omo", "Oh My OpenCode", {"agents": {}}, None, "omo", {}, 0),
    ("openclaw", "kimi-claw", "Kimi", {"baseUrl": "https://api.moonshot.cn/v1", "apiKey": "sk-test-claw", "api": "openai-completions", "models": [{"id": "kimi-k3", "name": "Kimi K3"}]}, None, "cn_official", {}, 0),
    ("hermes", "kimi-hermes", "Kimi", {"name": "kimi", "base_url": "https://api.moonshot.cn/v1", "api_key": "sk-test-hermes", "api_mode": "chat_completions", "models": [{"id": "kimi-k3", "name": "Kimi K3"}]}, None, "cn_official", {}, 0),
    ("pi", "kimi-pi", "Kimi", {"name": "Kimi", "baseUrl": "https://api.moonshot.cn/v1", "api": "openai-completions", "apiKey": "sk-test-pi", "models": [{"id": "kimi-k3", "name": "Kimi K3"}]}, None, "cn_official", {}, 0),
    ("mcode", "kimi-mcode", "Kimi", {"name": "Kimi", "kind": "custom", "enabled": True, "api": "openai-completions", "options": {"baseURL": "https://api.moonshot.cn/v1", "apiKey": "sk-test-mcode"}, "models": {"kimi-k3": {"name": "Kimi K3"}}}, None, "cn_official", {}, 0),
    ("copilot-cli", "x", "Some future app", {"env": {}}, None, None, {}, 0),
]

def v4():
    out = ["-- CC Switch SQLite 导出", "-- 生成时间: 2026-10-01 08:00:00", "-- user_version: 12",
           "PRAGMA foreign_keys=OFF;", "PRAGMA user_version=12;", "BEGIN TRANSACTION;"]
    out += [PROVIDERS_V4 + ";"] + [t + ";" for t in OTHER_TABLES]
    cols = "\"id\", \"app_type\", \"name\", \"settings_config\", \"website_url\", \"category\", \"created_at\", \"sort_index\", \"notes\", \"icon\", \"icon_color\", \"meta\", \"is_current\", \"in_failover_queue\""
    vals = []
    for i, (app, pid, name, settings, web, cat, meta, cur) in enumerate(ROWS):
        vals.append("(" + ", ".join(q(x) for x in [pid, app, name, settings, web, cat, 1759300000000 + i, i, None, None, None, meta, cur, 0]) + ")")
    out.append(f"INSERT INTO \"providers\" ({cols}) VALUES " + ",".join(vals) + ";")
    out.append("INSERT INTO \"provider_endpoints\" (\"id\", \"provider_id\", \"app_type\", \"url\", \"added_at\") VALUES (1, 'relay', 'claude', 'https://relay.example.com', 1759300000000);")
    out.append("INSERT INTO \"settings\" (\"key\", \"value\") VALUES ('codex_oauth_cache', '{\"refresh_token\":\"settings-secret\"}'),('language', 'zh');")
    out.append("INSERT INTO \"mcp_servers\" (\"id\", \"name\", \"server_config\", \"description\", \"homepage\", \"docs\", \"tags\", \"enabled_claude\", \"enabled_codex\") VALUES ('fs', 'fs', '{\"command\":\"npx\"}', NULL, NULL, NULL, '[]', 1, 0);")
    out.append("INSERT INTO sqlite_sequence (name, seq) VALUES ('provider_endpoints', 1);")
    out.append("COMMIT;")
    out.append("PRAGMA foreign_keys=ON;")
    return "\n".join(out) + "\n"

def v38():
    # 早期导出：每行一条 INSERT，providers 表还没有 icon / icon_color / in_failover_queue。
    out = ["-- CC Switch SQLite 导出", "-- 生成时间: 2025-11-20 10:00:00", "-- user_version: 1",
           "PRAGMA foreign_keys=OFF;", "PRAGMA user_version=1;", "BEGIN TRANSACTION;",
           """CREATE TABLE providers (
                id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
                settings_config TEXT NOT NULL, website_url TEXT, category TEXT,
                created_at INTEGER, sort_index INTEGER, notes TEXT,
                meta TEXT NOT NULL DEFAULT '{}', is_current BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            );""", "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);"]
    for app, pid, name, settings, meta in [
        ("claude", "old-relay", "Old relay", {"env": {"ANTHROPIC_BASE_URL": "https://old.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-old"}}, {}),
        ("codex", "old-codex", "Old codex", {"auth": {"OPENAI_API_KEY": "sk-test-old-codex"}, "config": CODEX_RESPONSES}, {}),
    ]:
        out.append("INSERT INTO \"providers\" (\"id\", \"app_type\", \"name\", \"settings_config\", \"website_url\", \"category\", \"created_at\", \"sort_index\", \"notes\", \"meta\", \"is_current\") VALUES ("
                   + ", ".join(q(x) for x in [pid, app, name, settings, None, "custom", 1732000000000, None, "it's old", meta, 0]) + ");")
    out += ["COMMIT;", "PRAGMA foreign_keys=ON;"]
    return "\n".join(out) + "\n"

def legacy_v2():
    return json.dumps({
        "version": 2,
        "claude": {"providers": {"json-relay": {"id": "json-relay", "name": "JSON relay", "settingsConfig": {"env": {"ANTHROPIC_BASE_URL": "https://j.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-json"}}, "websiteUrl": "https://j.example.com"}}, "current": "json-relay"},
        "codex": {"providers": {"json-codex": {"name": "JSON codex", "settingsConfig": {"auth": {"OPENAI_API_KEY": "sk-test-json-codex"}, "config": CODEX_RESPONSES}}}, "current": "json-codex"},
        "mcp": {"claude": {"servers": {}}},
    }, ensure_ascii=False, indent=2) + "\n"

def legacy_v1():
    return json.dumps({"providers": {"v1": {"id": "v1", "name": "V1 relay", "settingsConfig": {"env": {"ANTHROPIC_BASE_URL": "https://v1.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-v1"}}}}, "current": "v1"}, indent=2) + "\n"

for name, body in {
    "cc-switch-export-20261001_080000.sql": v4(),
    "cc-switch-export-v3.8.sql": v38(),
    "config-v2.json": legacy_v2(),
    "config-v1.json": legacy_v1(),
}.items():
    with open(os.path.join(HERE, name), "w", encoding="utf-8") as f:
        f.write(body)
