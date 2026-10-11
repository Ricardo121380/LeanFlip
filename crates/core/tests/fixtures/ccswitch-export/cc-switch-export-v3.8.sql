-- CC Switch SQLite 导出
-- 生成时间: 2025-11-20 10:00:00
-- user_version: 1
PRAGMA foreign_keys=OFF;
PRAGMA user_version=1;
BEGIN TRANSACTION;
CREATE TABLE providers (
                id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
                settings_config TEXT NOT NULL, website_url TEXT, category TEXT,
                created_at INTEGER, sort_index INTEGER, notes TEXT,
                meta TEXT NOT NULL DEFAULT '{}', is_current BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            );
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
INSERT INTO "providers" ("id", "app_type", "name", "settings_config", "website_url", "category", "created_at", "sort_index", "notes", "meta", "is_current") VALUES ('old-relay', 'claude', 'Old relay', '{"env":{"ANTHROPIC_BASE_URL":"https://old.example.com","ANTHROPIC_AUTH_TOKEN":"sk-test-old"}}', NULL, 'custom', 1732000000000, NULL, 'it''s old', '{}', 0);
INSERT INTO "providers" ("id", "app_type", "name", "settings_config", "website_url", "category", "created_at", "sort_index", "notes", "meta", "is_current") VALUES ('old-codex', 'codex', 'Old codex', '{"auth":{"OPENAI_API_KEY":"sk-test-old-codex"},"config":"model_provider = \"custom\"\nmodel = \"gpt-5.5\"\n\n[model_providers.custom]\nname = \"relay\"\nbase_url = \"https://relay.example.com/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"}', NULL, 'custom', 1732000000000, NULL, 'it''s old', '{}', 0);
COMMIT;
PRAGMA foreign_keys=ON;
