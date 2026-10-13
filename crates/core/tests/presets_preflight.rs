//! 前端预设（`ui/src/presets/*.json`）填上 key 以后要能通过切换前的校验。

use std::path::Path;

use leanflip_core::{store::StoreData, switch, Provider, Tool};
use serde_json::Value;

fn presets(tool: &str) -> Vec<Value> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../ui/src/presets/{tool}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn grok_presets_pass_preflight_once_a_key_is_filled_in() {
    let data = StoreData::default();
    let list = presets("grokbuild");
    assert!(list.iter().any(|p| p["isOfficial"] == true));
    assert!(list.iter().any(|p| p["isOfficial"] != true));
    for preset in list {
        let name = preset["name"].as_str().unwrap();
        let mut settings = preset["settingsConfig"].clone();
        let config = settings["config"]
            .as_str()
            .unwrap()
            .replace("api_key = \"\"", "api_key = \"sk-test\"");
        settings["config"] = Value::String(config);
        let mut provider = Provider::new("p", name, settings);
        if preset["isOfficial"] == true || preset["category"] == "official" {
            provider = provider.official();
        }
        switch::preflight(&data, Tool::GrokBuild, &provider)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
