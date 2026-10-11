-- CC Switch SQLite 导出
-- 生成时间: 2026-10-01 08:00:00
-- user_version: 12
PRAGMA foreign_keys=OFF;
PRAGMA user_version=12;
BEGIN TRANSACTION;
CREATE TABLE providers (
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
            );
CREATE TABLE provider_endpoints (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                url TEXT NOT NULL,
                added_at INTEGER,
                FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
            );
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, server_config TEXT NOT NULL,
            description TEXT, homepage TEXT, docs TEXT, tags TEXT NOT NULL DEFAULT '[]',
            enabled_claude BOOLEAN NOT NULL DEFAULT 0, enabled_codex BOOLEAN NOT NULL DEFAULT 0
        );
INSERT INTO "providers" ("id", "app_type", "name", "settings_config", "website_url", "category", "created_at", "sort_index", "notes", "icon", "icon_color", "meta", "is_current", "in_failover_queue") VALUES ('relay', 'claude', 'Relay', '{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.com","ANTHROPIC_AUTH_TOKEN":"sk-test-relay"}}', 'https://relay.example.com', 'third_party', 1759300000000, 0, NULL, NULL, NULL, '{}', 1, 0),('kimi', 'claude', 'Kimi', '{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-test-kimi"}}', 'https://platform.kimi.com?track_id=t-1&aff=cc-switch', 'cn_official', 1759300000001, 1, NULL, NULL, NULL, '{"isPartner":true,"partnerPromotionKey":"kimi","usageScript":{"enabled":true,"code":"return 1","accessToken":"usage-secret"}}', 0, 0),('openai-chat', 'claude', 'OpenAI via proxy', '{"env":{"ANTHROPIC_BASE_URL":"https://o.example.com"}}', NULL, 'custom', 1759300000002, 2, NULL, NULL, NULL, '{"apiFormat":"openai_chat"}', 0, 0),('taken', 'claude', 'Taken id', '{"env":{"ANTHROPIC_BASE_URL":"https://another.example.com","ANTHROPIC_AUTH_TOKEN":"sk-test-other"}}', NULL, 'custom', 1759300000003, 3, NULL, NULL, NULL, '{}', 0, 0),('same-as-mine', 'claude', 'Copy of mine', '{"env":{"ANTHROPIC_BASE_URL":"https://mine.example.com","ANTHROPIC_AUTH_TOKEN":"sk-test-mine"}}', NULL, 'custom', 1759300000004, 4, NULL, NULL, NULL, '{}', 0, 0),('claude-official', 'claude', 'Claude Official', '{"env":{"CLAUDE_CODE_OAUTH_TOKEN":"oauth-secret"}}', 'https://www.anthropic.com/claude-code', 'official', 1759300000005, 5, NULL, NULL, NULL, '{}', 0, 0),('desk', 'claude-desktop', 'Desktop relay', '{"env":{"ANTHROPIC_BASE_URL":"https://desk.example.com","ANTHROPIC_AUTH_TOKEN":"sk-test-desk"}}', NULL, 'third_party', 1759300000006, 6, NULL, NULL, NULL, '{"claudeDesktopModelRoutes":{"claude-sonnet-4-5":{"model":"","labelOverride":"Sonnet"}}}', 1, 0),('desk-proxy', 'claude-desktop', 'Desktop proxy', '{"env":{"ANTHROPIC_BASE_URL":"https://p.example.com","ANTHROPIC_AUTH_TOKEN":"sk-test-p"}}', NULL, 'third_party', 1759300000007, 7, NULL, NULL, NULL, '{"claudeDesktopMode":"proxy"}', 0, 0),('official', 'codex', 'OpenAI Official', '{"auth":{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"id-secret","access_token":"at-secret","refresh_token":"rt-secret","account_id":"acct-1"},"last_refresh":"2026-01-01T00:00:00Z"},"config":""}', 'https://chatgpt.com/codex', 'official', 1759300000008, 8, NULL, NULL, NULL, '{"authBinding":{"authProvider":"codex_oauth","accountId":"acct-1"}}', 1, 0),('codex-relay', 'codex', 'Codex relay', '{"auth":{"OPENAI_API_KEY":"sk-test-codex"},"config":"model_provider = \"custom\"\nmodel = \"gpt-5.5\"\n\n[model_providers.custom]\nname = \"relay\"\nbase_url = \"https://relay.example.com/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"}', 'https://relay.example.com/r/ABC123', 'third_party', 1759300000009, 9, NULL, NULL, NULL, '{}', 0, 0),('copilot', 'codex', 'GitHub Copilot', '{"auth":{},"config":""}', NULL, 'third_party', 1759300000010, 10, NULL, NULL, NULL, '{"providerType":"github_copilot"}', 0, 0),('gem', 'gemini', 'Gemini relay', '{"env":{"GOOGLE_GEMINI_BASE_URL":"https://g.example.com","GEMINI_API_KEY":"sk-test-gem","GEMINI_MODEL":"gemini-3.6-flash"}}', NULL, 'third_party', 1759300000011, 11, NULL, NULL, NULL, '{}', 1, 0),('grok-relay', 'grokbuild', 'Grok relay', '{"config":"[models]\ndefault = \"grok-4.5\"\n\n[model.\"grok-4.5\"]\nmodel = \"grok-4.5\"\nbase_url = \"https://grok-relay.example.com/v1\"\nname = \"relay\"\napi_key = \"sk-test-grok\"\napi_backend = \"responses\"\ncontext_window = 500000\n"}', NULL, 'third_party', 1759300000012, 12, NULL, NULL, NULL, '{}', 0, 0),('grok-oauth', 'grokbuild', 'Grok (xAI login)', '{"auth":{},"config":""}', NULL, 'official', 1759300000013, 13, NULL, NULL, NULL, '{"providerType":"xai_oauth"}', 1, 0),('kimi-oc', 'opencode', 'Kimi', '{"npm":"@ai-sdk/openai-compatible","name":"Kimi","options":{"baseURL":"https://api.moonshot.cn/v1","apiKey":"sk-test-oc"},"models":{"kimi-k3":{"name":"Kimi K3"}}}', NULL, 'cn_official', 1759300000014, 14, NULL, NULL, NULL, '{}', 0, 0),('omo', 'opencode', 'Oh My OpenCode', '{"agents":{}}', NULL, 'omo', 1759300000015, 15, NULL, NULL, NULL, '{}', 0, 0),('kimi-claw', 'openclaw', 'Kimi', '{"baseUrl":"https://api.moonshot.cn/v1","apiKey":"sk-test-claw","api":"openai-completions","models":[{"id":"kimi-k3","name":"Kimi K3"}]}', NULL, 'cn_official', 1759300000016, 16, NULL, NULL, NULL, '{}', 0, 0),('kimi-hermes', 'hermes', 'Kimi', '{"name":"kimi","base_url":"https://api.moonshot.cn/v1","api_key":"sk-test-hermes","api_mode":"chat_completions","models":[{"id":"kimi-k3","name":"Kimi K3"}]}', NULL, 'cn_official', 1759300000017, 17, NULL, NULL, NULL, '{}', 0, 0),('kimi-pi', 'pi', 'Kimi', '{"name":"Kimi","baseUrl":"https://api.moonshot.cn/v1","api":"openai-completions","apiKey":"sk-test-pi","models":[{"id":"kimi-k3","name":"Kimi K3"}]}', NULL, 'cn_official', 1759300000018, 18, NULL, NULL, NULL, '{}', 0, 0),('kimi-mcode', 'mcode', 'Kimi', '{"name":"Kimi","kind":"custom","enabled":true,"api":"openai-completions","options":{"baseURL":"https://api.moonshot.cn/v1","apiKey":"sk-test-mcode"},"models":{"kimi-k3":{"name":"Kimi K3"}}}', NULL, 'cn_official', 1759300000019, 19, NULL, NULL, NULL, '{}', 0, 0),('x', 'copilot-cli', 'Some future app', '{"env":{}}', NULL, NULL, 1759300000020, 20, NULL, NULL, NULL, '{}', 0, 0);
INSERT INTO "provider_endpoints" ("id", "provider_id", "app_type", "url", "added_at") VALUES (1, 'relay', 'claude', 'https://relay.example.com', 1759300000000);
INSERT INTO "settings" ("key", "value") VALUES ('codex_oauth_cache', '{"refresh_token":"settings-secret"}'),('language', 'zh');
INSERT INTO "mcp_servers" ("id", "name", "server_config", "description", "homepage", "docs", "tags", "enabled_claude", "enabled_codex") VALUES ('fs', 'fs', '{"command":"npx"}', NULL, NULL, NULL, '[]', 1, 0);
INSERT INTO sqlite_sequence (name, seq) VALUES ('provider_endpoints', 1);
COMMIT;
PRAGMA foreign_keys=ON;
