# ccsl CLI

`ccsl` 与桌面 App 共用 `ccsl-service`、供应商库和目录覆盖设置，不需要运行桌面 App 或安装 WebView。

```sh
cargo build -p ccsl-cli --release
./target/release/ccsl status
./target/release/ccsl list codex
./target/release/ccsl paths codex
./target/release/ccsl use codex provider-id
./target/release/ccsl enable opencode provider-id
./target/release/ccsl disable opencode provider-id
```

工具名：`claude`、`codex`、`gemini`、`grokbuild`、`claude-desktop`、`opencode`、`openclaw`、`hermes`、`pi`、`mcode`。后五种使用 `enable` / `disable`，其余使用 `use`。

`status` 显示保存在卡片库里的当前选择；它不证明客户端文件未经外部修改。`list` 的 `liveReadable` 为 `false` 时，`enabled` 状态不可用。查询及导入预览不写配置、不执行启动恢复；需要手动恢复中断写入时执行 `ccsl recover [tool]`。修改命令会先处理相应工具的中断写入。

## 导入与卡片编辑

```sh
ccsl import live claude
ccsl import file backup.sql                         # 默认只预览
ccsl import file backup.sql --preview --tool codex
ccsl import file backup.sql --apply --duplicate skip
ccsl import file backup.sql --apply --duplicate rename --tool claude,codex
ccsl provider add claude --file provider.json
ccsl provider add claude --file provider.json --activate
ccsl provider update claude --file provider.json
ccsl provider delete claude provider-id
cat provider.json | ccsl provider add claude          # 默认读 stdin
```

导出文件支持 SQL、SQLite 和旧版 JSON，`--apply` 只写卡片库，不激活供应商。`--duplicate rename` 对 ID 相同而配置不同的卡片生成新 ID；配置相同的重复项仍跳过。

单个卡片输入采用 App 原有的 Provider JSON 格式，至少包含 `id`、`name`、`settingsConfig`，最大 1 MiB。例如 Claude 卡片：

```json
{
  "id": "relay",
  "name": "My provider",
  "settingsConfig": {
    "env": {
      "ANTHROPIC_BASE_URL": "https://api.example.com",
      "ANTHROPIC_AUTH_TOKEN": "replace-in-private-file"
    }
  }
}
```

请通过私有文件或标准输入提供密钥，避免把凭据放进命令参数和 shell 历史。`provider update` 是整张卡片替换，修改生效中的卡片也会同步客户端配置。并存式卡片删除时会同步移出客户端配置；切换式当前卡片需先切换到其他卡片。

## Codex 账号

```sh
ccsl accounts list
ccsl accounts login --timeout 300
ccsl accounts login --account account-id
ccsl accounts default account-id
ccsl accounts refresh account-id
ccsl accounts remove account-id
```

设备登录显示验证网址和短验证码，用户在浏览器中确认；默认最多等待 300 秒，`--timeout` 支持 1–1800 秒，单次网络请求另有 30 秒超时。CLI 不输出 device code、access token、refresh token 或 ID token。`refresh` 强制刷新指定账号的 token；账号库保存更新后的凭据，客户端正在使用该账号时尝试同步登录文件。`remove` 只删除本地托管账号，不在 OpenAI 服务端注销或撤销账号。

## 脚本接口与隔离环境

`--json` 和 `--home` 是全局参数，可写在子命令前后：

```sh
ccsl status --json
ccsl --json list codex
ccsl --home /absolute/path/to/test-home paths
```

默认使用 App 相同的系统路径、环境变量和 `app-settings.json` 目录覆盖。`--home` 使用指定根目录的默认布局，忽略 `CODEX_HOME` 等各工具环境变量，但仍应用该根目录下保存的 App 目录覆盖；需要完全隔离的测试环境时使用空目录。

标准输出成功格式为 `{"ok":true,"data":...}`；错误只写标准错误，格式为 `{"ok":false,"error":{"code":"...","message":"..."}}`。退出码 `0` 表示成功，`2` 表示参数或输入错误，`1` 表示执行失败。错误 `code` 保留业务层稳定键；说明不会回显配置原文。设备登录在标准错误输出额外的 `device_login` JSON 事件，最终结果仍在标准输出。

列表仅输出卡片身份、当前选择和启用状态，不输出 `settingsConfig`、扩展字段或 `meta`。导入预览仅输出原因代码；操作警告和恢复结果输出数量，避免解析错误或恢复信息中的配置内容泄漏。数量非零时，请通过 `paths` 定位并检查相关配置。

验证：`cargo test -p ccsl-cli`。所有子进程测试均使用独立临时目录，不触碰真实客户端配置或发起外部登录。
