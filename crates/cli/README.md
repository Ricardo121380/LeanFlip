# ccsl CLI

`ccsl` 与桌面 App 共用 `ccsl-service`、供应商库和目录覆盖设置，不需要运行桌面 App 或安装 WebView。

下载 [GitHub Release](https://github.com/Ricardo121380/ccswitch-lite/releases/latest) 中对应平台的
`ccsl_*.tar.gz`，解压后直接运行 `./ccsl`；需要全局调用时，将二进制放入已有的 PATH 目录。
预编译版本无需安装 Rust 或 Node.js。以下命令适用于从源码构建：

```sh
cargo build -p ccsl-cli --release
./target/release/ccsl
./target/release/ccsl menu codex
./target/release/ccsl tui
./target/release/ccsl tui codex
./target/release/ccsl status
./target/release/ccsl list codex
./target/release/ccsl paths codex
./target/release/ccsl use codex provider-id
./target/release/ccsl enable opencode provider-id
./target/release/ccsl disable opencode provider-id
./target/release/ccsl --lang en status
```

终端界面默认中文，`--lang en` 切换英文，帮助与错误提示也随之切换。`ccsl --help` 和各子命令的 `--help` 包含可参照的示例。列表按工具模式显示「当前使用」或「已启用」，并给出下一步命令；空列表和无法读取的配置有单独提示。表格按 Unicode 显示宽度对齐，超出 `COLUMNS`（默认 100 列）时显示为字段列表，供应商与账号 ID 始终完整保留。

在交互终端中直接运行 `ccsl` 会打开菜单，也可以显式使用 `ccsl menu`（别名 `ccsl ui`）。先选择工具，再选择供应商；输入文字可筛选，方向键移动，回车确认，Esc 返回或退出，Ctrl+C 退出。执行前会展示完整目标及配置路径，取消不会修改配置。菜单提供供应商切换、并存工具启用/停用、导入当前客户端配置和查看路径；状态无法读取时会阻止启用/停用。

`ccsl menu codex` 可直接进入指定工具，菜单同样支持 `--lang en` 和 `--home`。菜单会刷新当前选择与启用状态，不显示密钥或登录令牌。普通命令继续适用于脚本；输入/输出被重定向时，无命令的 `ccsl` 保持显示快速入门，显式 `menu` 则提示需要交互终端。`--json` 不进入菜单。

## 全屏 TUI

`ccsl tui [tool]` 是独立的全屏工作台，支持 `--lang en` 和 `--home`。当前已发布的 v0.1.0 二进制尚未包含此功能，请按上面的源码命令构建；直接运行 `ccsl` 的默认菜单保持不变。

- `Tab` / `←` / `→` 切换工具与供应商区域，`↑` / `↓` 或 `j` / `k` 移动。
- `/` 搜索名称或 ID，`Enter` 完成搜索；再次 `Enter` 预览操作。确认默认取消，`Tab` 后回车或 `y` / `Y` 才执行。
- `i` 导入当前客户端配置；`r` 重新读取最新状态；`p` / `d` 查看完整详情与路径；`?` / `F1` 查看帮助。
- `Esc` 清除筛选、返回或关闭面板；`q` 退出（弹窗中先关闭）；`Ctrl+C` 退出并恢复终端。正在执行的写入会先完成再响应中断。
- 宽窗口三栏、普通窗口两栏、窄窗口单栏；面板长内容用 `↑↓` / `PgUp` / `PgDn` 滚动。小于 44×12 时暂停操作并提示扩大窗口。

TUI 不显示配置内容或凭据；浏览不写入，状态需按 `r` 刷新。确认前重新读取卡片和状态，变化时拒绝执行旧确认；并存配置不可读时阻止启用/停用。卡片编辑、文件导入和账号登录使用普通命令。管道或 `--json` 不启动 TUI。

`ccsl provider` 等未选择动作的命令组会显示对应帮助，不读取客户端配置。参数填写错误时，终端显示已识别命令的用法、示例和合法工具名，不回显输入原文。人读输出可配合 `head` 等管道工具使用：接收端提前关闭时安静退出，不把截断阅读视为操作失败。

工具名：`claude`、`codex`、`gemini`、`grokbuild`、`claude-desktop`、`opencode`、`openclaw`、`hermes`、`pi`、`mcode`。后五种使用 `enable` / `disable`，其余使用 `use`。

开发时运行 `cargo test -p ccsl-cli --locked`。macOS/Linux 使用 `python3 scripts/check-cli-menu.py --binary target/release/ccsl` 和 `python3 scripts/check-cli-tui.py --binary target/release/ccsl` 在真实伪终端中验证选择、筛选、取消、写入及中断后的终端恢复；这些检查仅使用临时 HOME，不访问实际账号。

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

`--json`、`--home` 和 `--lang` 是全局参数，可写在子命令前后：

```sh
ccsl status --json
ccsl --json list codex
ccsl --home /absolute/path/to/test-home paths
ccsl list codex --lang en
```

默认使用 App 相同的系统路径、环境变量和 `app-settings.json` 目录覆盖。`--home` 使用指定根目录的默认布局，忽略 `CODEX_HOME` 等各工具环境变量，但仍应用该根目录下保存的 App 目录覆盖；需要完全隔离的测试环境时使用空目录。

标准输出成功格式为 `{"ok":true,"data":...}`；错误只写标准错误，格式为 `{"ok":false,"error":{"code":"...","message":"..."}}`。退出码 `0` 表示成功，`2` 表示参数或输入错误，`1` 表示执行失败。错误 `code` 保留业务层稳定键；说明不会回显配置原文。`--lang` 只改变人读界面，不改变 JSON 字段、英文错误消息或退出码；脚本应使用 `--json`。设备登录在标准错误输出额外的 `device_login` JSON 事件，最终结果仍在标准输出。

`--json` 模式没有快速入门旁路：缺少命令仍返回 `invalid_arguments` 和退出码 `2`，输出管道提前关闭仍返回退出码 `1`，保证脚本兼容。

列表仅输出卡片身份、当前选择和启用状态，不输出 `settingsConfig`、扩展字段或 `meta`。导入预览在终端显示固定的原因说明，JSON 保留原因代码；操作警告和恢复结果输出数量，避免解析错误或恢复信息中的配置内容泄漏。数量非零时，请通过 `paths` 定位并检查相关配置。

切换式工具使用 `current` 表示库中选择，`enabled` / `inLive` / `liveReadable` 为 `null`；
并存式工具配置读不出来时，启用状态也为 `null`，不会假报为全部关闭。
若刷新返回 `codex.oauth.live_sync_failed`，新凭据已经保存在账号库，只有客户端同步失败；
修复配置路径或权限后再操作，不要用旧 token 覆盖新凭据。

验证：`cargo test -p ccsl-cli`。所有子进程测试均使用独立临时目录，不触碰真实客户端配置或发起外部登录。
