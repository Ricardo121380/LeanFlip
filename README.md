# LeanFlip · 简切

**一份配置库，桌面 App、命令行和全屏 TUI 三种用法。**

LeanFlip 把 AI 编程工具的供应商配置保存在本地，需要时写进 Claude Code、Codex、Gemini CLI 等工具实际读取的配置文件。App、命令行 `leanflip` 和 TUI 共用同一份数据：可以先在 App 里添加供应商，再在终端里用方向键切换。

LeanFlip 只管理配置，不提供本地代理、请求转发、负载均衡或用量统计。使用前需要自行安装对应的 AI 工具，并准备该工具支持的账号或 API Key。

[从源码构建](#安装) · [CLI 完整说明](crates/cli/README.md) · [反馈问题](https://github.com/Ricardo121380/LeanFlip/issues)

> **版本说明：** 当前源码版本为 `0.2.0-dev`，尚未发布预编译安装包，请按下文从源码构建。旧版本的数据目录会在首次使用时自动迁移到 `~/.leanflip/`，供应商、托管账号和设置都会保留。

## 目录

- [选择 App、命令行还是 TUI](#选择-app命令行还是-tui)
- [安装](#安装)
- [第一次使用](#第一次使用)
- [支持的工具与操作方式](#支持的工具与操作方式)
- [桌面 App 使用指南](#桌面-app-使用指南)
- [CLI 使用指南](#cli-使用指南)
- [从 CC Switch 迁移](#从-cc-switch-迁移)
- [加密完整备份与恢复](#加密完整备份与恢复)
- [Codex 登录与多账号](#codex-登录与多账号)
- [配置目录与数据备份](#配置目录与数据备份)
- [常见问题](#常见问题)
- [升级与卸载](#升级与卸载)
- [开发](#开发)
- [项目结构与许可证](#项目结构与许可证)

## 选择 App、命令行还是 TUI

| 使用场景 | 建议入口 |
| --- | --- |
| 第一次配置，希望从预设填写地址、密钥和模型 | 桌面 App |
| 用键盘快速切换任意工具的供应商 | App 里按 `⌘K`（Windows / Linux 为 `Ctrl+K`） |
| 希望用方向键选择工具、搜索供应商、回车执行 | 交互菜单：`leanflip` |
| 希望在全屏终端中浏览工具、供应商和详情 | [TUI 工作台](#全屏-tui-工作台)：`leanflip tui` |
| 通过 SSH 使用，或在没有桌面环境的机器上管理配置 | 命令行 |
| 在脚本中查询状态或切换供应商 | 命令模式，配合 `--json` |
| 从本机 CC Switch 迁移供应商及托管账号 | App“数据与迁移”、`leanflip import local` 或 TUI 数据中心 |
| 在设备之间迁移供应商、托管账号及偏好 | 三端通用的密码加密 `.leanflip` 备份 |

App 和命令行可以单独使用，也可以一起使用。命令行不需要 Node.js、WebView 或桌面 App。

## 安装

预编译安装包会随下一个正式版本发布。在此之前，请从源码构建。

### 构建命令行与 TUI

需要 Rust stable 和平台 C/C++ 构建工具；工作区声明的最低 Rust 版本为 `1.89`。

```sh
git clone https://github.com/Ricardo121380/LeanFlip.git
cd LeanFlip
cargo build --release --locked --bin leanflip
./target/release/leanflip --version
```

该命令只构建命令行，不需要 Tauri 桌面依赖或前端依赖。希望在任何目录直接输入 `leanflip`，可以安装到当前用户的命令目录：

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 target/release/leanflip "$HOME/.local/bin/leanflip"
export PATH="$HOME/.local/bin:$PATH"
```

上面的 `export` 只对当前终端生效。要让新终端也能找到命令，把 `export PATH="$HOME/.local/bin:$PATH"` 加入 Shell 启动文件，例如 zsh 的 `~/.zshrc` 或交互式 bash 的 `~/.bashrc`。

### 构建桌面 App

除了 Rust，还需要 Node.js、pnpm 以及 Tauri 的系统依赖。CI 使用 **Node.js 24** 和 **pnpm 11.22.0**；macOS 需要 Xcode Command Line Tools。Ubuntu 24.04 的桌面依赖与 CI 一致：

```sh
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential libssl-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

在仓库根目录执行：

```sh
pnpm install --frozen-lockfile --ignore-scripts

# macOS：生成 LeanFlip.app 和 DMG
CI=true pnpm tauri build --ci --bundles app,dmg -- --locked

# Linux：生成 DEB
pnpm tauri build --ci --bundles deb -- --locked
```

产物位于 `target/release/bundle/`。macOS 上把 `LeanFlip.app` 拖入“应用程序”即可；Linux 使用 `sudo apt install ./生成的文件.deb`。

## 第一次使用

### 已经在使用某个 AI 工具

1. 在 App 左侧选择对应工具，例如 **Claude Code** 或 **Codex**。
2. 确认页面顶部显示的配置文件路径就是你实际使用的位置；自定义目录可在“设置 → 配置目录”中填写。
3. 点击“导入当前配置”，把客户端现有配置保存为一个供应商。
4. 根据需要添加其他供应商，然后点击“切换”，或打开并存式工具的开关。
5. 回到对应 AI 工具，重新打开会话或重新加载配置，确认实际使用的供应商和模型。

命令行中对应的流程：

```sh
leanflip paths codex          # 先确认配置路径
leanflip import live codex    # 导入当前客户端配置
leanflip menu codex           # 在菜单中选择供应商
```

“导入当前配置”主要用于初次接管：**切换式工具已经有供应商时，不会重新导入或覆盖它们**；并存式工具可以重复导入，补齐尚未保存的条目。导入本身不切换供应商；如果上次有中断的写入，写操作会先处理恢复，详见[中断恢复](#中断恢复)。

### 还没有供应商配置

1. 在 App 中选择工具，点击“添加供应商”（工具还没有供应商时是“从预设添加”）。
2. 选择一个起点：适合的预设，或“空白配置”。
3. 在“常用设置”里填写名称、请求地址、API Key 和模型。可以点“获取模型列表”从供应商拉取可选模型，用“测连通”“检查模型”确认地址和密钥可用。预设只提供配置结构，实际账号和可用模型由供应商提供。
4. 保存。需要立即使用时，勾选“保存后立即切换到它”或“保存后立即写入客户端配置”。

只使用命令行时，可以通过 JSON 文件[新增供应商](#新增和更新供应商)。

## 支持的工具与操作方式

**供应商**是 LeanFlip 保存的一组配置；**客户端配置**是 AI 工具实际读取的文件。新增的供应商默认只保存在库中，切换或启用后才会写入客户端。

| 工具 | 命令行工具名 | 模式 | 常用操作 |
| --- | --- | --- | --- |
| Claude Code | `claude` | 切换，一次一个 | `leanflip use claude PROVIDER_ID` |
| Codex | `codex` | 切换，一次一个 | `leanflip use codex PROVIDER_ID` |
| Gemini CLI | `gemini` | 切换，一次一个 | `leanflip use gemini PROVIDER_ID` |
| Grok Build | `grokbuild` | 切换，一次一个 | `leanflip use grokbuild PROVIDER_ID` |
| Claude Desktop | `claude-desktop` | 切换，一次一个 | `leanflip use claude-desktop PROVIDER_ID` |
| OpenCode | `opencode` | 并存，可同时启用多个 | `leanflip enable opencode PROVIDER_ID` |
| OpenClaw | `openclaw` | 并存，可同时启用多个 | `leanflip enable openclaw PROVIDER_ID` |
| Hermes | `hermes` | 并存，可同时启用多个 | `leanflip enable hermes PROVIDER_ID` |
| Pi | `pi` | 并存，可同时启用多个 | `leanflip enable pi PROVIDER_ID` |
| MCode | `mcode` | 并存，可同时启用多个 | `leanflip enable mcode PROVIDER_ID` |

`PROVIDER_ID` 是占位符，请替换为 `leanflip list 工具名` 中显示的真实 ID。名称用于展示，ID 用于命令和配置条目定位；创建后不能修改 ID。

- **切换式工具**：同一时间只用一个供应商，使用 `use` 切换。
- **并存式工具**：多个供应商可以同时写入客户端，使用 `enable` / `disable` 启用或停用。具体使用哪个模型，仍由客户端自己的模型选择决定。
- **Hermes**：App 还提供“设为默认模型”，与供应商条目的启用开关分开。
- **Claude Desktop**：支持直连配置；模型菜单要求 `claude-sonnet-*`、`claude-opus-*`、`claude-haiku-*` 等受支持的官方模型名，不提供代理模式或模型名映射。

## 桌面 App 使用指南

### 首页

打开 App 先看到首页，每个工具一句话，例如“Claude Code 正在用 Kimi”：

- 点供应商名，其他供应商在同一行展开，选一个即切换；成功提示里可以一键切回。按 `Esc` 或点别处收起。
- 并存式工具的句子里，点灰色的名字写入客户端配置，点彩色的名字移出；提示里可以撤销。
- 点工具名进入它的工具页面；本机没有检测到的工具集中列在最后一行。
- 右上角的“清单 / 光路”切换视图。光路左边是工具，右边是按地址合并的服务端点，正在使用的路由上有光点流动；点一条暗线即切换到那个供应商。
- “测连通”逐个访问所有供应商的请求地址，延迟显示在供应商名旁边和光路的端点下方。这一步不发送密钥。

系统开启“减少动态效果”时，界面直接显示结果，不播放动画。

### 统计

侧栏“统计”显示每天用了多少 token。LeanFlip 不经手请求，数字来自各工具自己写在本机的会话记录：只读取时间、模型、供应商和 token 数，不读对话内容，也不上传。

- 选择最近 7 / 30 / 90 天和工具，汇总、时间线、每日用量和供应商明细一起变化。
- 时间线显示每段时间用的是哪个供应商。点过去的一段，即切回那个供应商。
- 供应商明细列出未缓存输入、缓存读取、缓存写入、输出和缓存命中率。有缓存写入、命中率却低于 50% 时会提示“偏低”，常见于缓存没有生效的中转站。
- 暂不计算费用：中转站价格各不相同，按官方价估算会误导。

| 工具 | 读取的记录 | 归到供应商 |
| --- | --- | --- |
| Claude Code | `~/.claude/projects/` | 按切换时间推算 |
| Codex | `~/.codex/sessions/`（含 `archived_sessions/`） | 会话的供应商 ID 只对应一个供应商时精确，否则按切换时间推算 |
| Gemini CLI | `~/.gemini/tmp/*/chats/`（未实测） | 按切换时间推算 |
| Grok Build | `~/.grok/sessions/*/*/updates.jsonl`（格式未公开） | 按切换时间推算 |
| OpenCode | `~/.local/share/opencode/opencode.db` 及旧版 `storage/message/` | 记录自带 |
| OpenClaw | `~/.openclaw/agents/`（未实测） | 记录自带 |
| Hermes | `~/.hermes/state.db`（未实测） | 记录自带 |
| Pi | `~/.pi/agent/sessions/`（未实测） | 记录自带 |

Claude Desktop 和 MCode 在本机不留用量记录。标“未实测”的格式来自公开资料，还没在真实数据上验证。

切换记录从这个版本开始积累：每次切换、写入、移出后记一条，打开统计页时也会对比一次当前状态。更早的用量只能显示为“开始记录之前”。按切换时间推算是近似值：切换前已经打开的会话可能被算到新供应商名下。

Claude Code 和 Gemini CLI 默认只保留 30 天的会话记录。LeanFlip 把读到的数字按天汇总进 `usage.db`，原始记录删掉后统计还在。

### 查看和切换供应商

左侧按“切换使用 · 一次一个”和“同时启用 · 可多个”分组，每个工具下方显示当前供应商（“→ 名称”）或已启用数量；标识变灰表示没有检测到该工具的配置。进入工具页面后，可以：

- 页面顶部的“当前路由”显示配置文件指向哪个供应商（地址和模型）；并存式工具显示已写入客户端的供应商。
- 下方列出全部供应商及其地址和模型，正在使用的一行带“使用中”标记。
- 点击某一行的“切换”立即切换；成功提示里可以一键切回上一个供应商。
- 并存式工具用开关写入或移出客户端配置。
- 按名称、ID、地址、模型或备注搜索；复制页面顶部显示的配置文件路径。
- 点击“测连通”，逐个访问各供应商的请求地址，在每一行显示延迟（500 ms 以内为绿色、1 秒以内为黄色、更慢或不可连通为红色）。这一步不发送密钥。
- 读取失败时重试；并存配置不可读时，界面会阻止不可靠的启用 / 停用操作。

“使用中”表示 LeanFlip 保存的选择，不是连通性检测结果，也不能证明配置文件没有被其他程序改动。

### 命令面板

按 `⌘K`（Windows / Linux 为 `Ctrl+K`），或点击侧栏顶部的搜索框，打开命令面板：

- 输入工具名、供应商名称、ID 或地址，回车即可切换供应商，或启用 / 停用并存式供应商。
- 也可以直接跳到任意工具、Codex 账号、数据与迁移或设置页面。
- `↑` / `↓` 选择，`Enter` 执行，`Esc` 关闭。

### 添加、编辑与删除

添加时先选择起点（预设或空白配置），再进入编辑器。编辑器分两页：

- **常用设置**：名称、ID、网站、备注，请求地址与 API Key（默认隐藏，可点眼睛显示），以及各工具的模型字段（如 Claude Code 的默认模型和 Haiku / Sonnet / Opus 模型、Codex 的推理强度、并存式工具的模型列表）。
- **完整配置**：原始 JSON；Codex 分别展示 `auth.json` 和 `config.toml`。两页实时同步，TOML 只改动对应的行，注释和顺序保持不变。Claude Code 会列出不随切换写入的字段。

每一行供应商都可以“复制”，生成一个以它为模板的新供应商（不复制备注和 ChatGPT 登录凭据）。

编辑器里的检测都使用正在编辑、尚未保存的配置，只在点击时运行：

| 操作 | 访问供应商的方式 | 是否发送密钥 |
| --- | --- | --- |
| 测连通 | 访问请求地址本身，收到任何 HTTP 响应都算连通 | 否 |
| 获取模型列表 | 请求供应商的模型列表接口（OpenAI 兼容的 `/models`、Anthropic 的 `/v1/models`、Gemini 的 `/v1beta/models`） | 是 |
| 检查模型 | 发送一次最短的对话请求，最多 16 个输出 token | 是 |

**推理强度**：Claude Code（写入 `CLAUDE_CODE_EFFORT_LEVEL`）和 Codex（写入 `model_reasoning_effort`）在“模型”一节用档位按钮选择，也可以自定义。可用档位有两个来源：

- 获取模型列表时，Anthropic 接口会给出每个模型支持的档位，LeanFlip 直接据此标出不可用的档位。
- 点“检测可用档位”，用当前模型先发一次不带档位的请求，再逐档各发一次（Claude 共 6 次，Codex 共 7 次，每次最多 16 个输出 token），被拒绝的档位标为不可用。OpenAI 兼容接口的模型列表不包含档位信息，只能这样检测。

通过网关使用自定义模型名时，Claude Code 默认不发送推理强度，可勾选“第三方模型名也发送推理强度”（`CLAUDE_CODE_ALWAYS_ENABLE_EFFORT=1`）。

检测不会跟随重定向，避免把密钥发往其他主机；错误说明中出现的密钥会被遮盖。使用 Bedrock / Vertex 等云厂商认证，或 Codex 内置本地供应商（ollama、lmstudio）时不提供检测。部分网关会拦截此类请求并返回 401 / 403，此时结果不一定代表客户端无法使用。检测请求遵循 `HTTPS_PROXY` / `HTTP_PROXY` / `NO_PROXY` 环境变量，但不读取 macOS「系统设置」里的代理；从访达启动的 App 通常没有这些环境变量，需要代理时请使用代理工具的增强（TUN）模式。

- 新增的供应商默认只保存；勾选立即应用后才会同时写入客户端。
- **编辑正在使用的供应商并保存，会同步更新客户端配置。**
- 关闭编辑器、更换预设或离开有未保存修改的设置页时，会询问是否放弃修改。
- 正在使用的切换式供应商需要先切换到其他供应商，才能删除。
- 删除并存式供应商时，也会从客户端移除对应条目；如果只是暂时停用，关闭开关即可保留它。

### 设置与托盘

| 设置 | 作用 |
| --- | --- |
| 界面语言 | 中文、英文或跟随系统；命令行使用自己的 `--lang` 参数 |
| 外观 | 浅色、深色或跟随系统 |
| 配置目录 | 覆盖对应客户端目录，只接受绝对路径；留空使用默认值，保存后生效 |
| 开机自动启动 | 登录系统时启动 App；默认关闭 |
| 关闭窗口时留在托盘 | 默认开启；关闭窗口后仍可通过托盘切换供应商 |
| 托管账号后台刷新 | 账号上次取得令牌超过设定小时数后进入刷新范围；默认 168 小时，`0` 关闭 |
| 切到第三方时保留 Codex 的 ChatGPT 登录 | 控制第三方配置与本机 ChatGPT 登录的保留方式，详见[Codex 登录与多账号](#codex-登录与多账号) |

保存设置后，目录覆盖会被 App 和之后的命令行共同使用。托盘菜单中可以打开主窗口、切换供应商或退出应用；退出 App 后，后台刷新也会停止。

## CLI 使用指南

### 全屏 TUI 工作台

`leanflip tui` 在一个全屏界面中浏览工具、搜索供应商、查看状态与配置路径，并确认切换、启用 / 停用或导入。

```sh
leanflip tui
leanflip tui codex            # 直接选中 Codex
leanflip tui --lang en       # 英文界面
```

宽窗口显示工具、供应商和详情三栏；普通窗口显示两栏，窄窗口按焦点显示单栏。按 `p` 可随时打开完整详情和路径，长内容支持滚动。终端小于 44 列、12 行时会提示扩大窗口，并暂停操作键。

| 按键 | 效果 |
| --- | --- |
| `Tab` / `←` / `→` | 切换工具与供应商区域 |
| `↑` / `↓` 或 `j` / `k` | 移动选择 |
| `Enter` | 选择工具，或预览所选供应商的操作 |
| `/` | 按名称、ID 或 Base URL 搜索；`Enter` 完成输入，`Esc` 清除 |
| `e` | 编辑所选供应商的名称、Base URL 和 API Key |
| `b` | 打开数据中心：本机迁移、文件导入与加密导出 |
| `i` | 预览从当前客户端配置导入 |
| `r` | 重新读取最新状态，保留仍存在的选择 |
| `p` / `d` | 完整详情与配置路径 |
| `?` / `F1` | 快捷键帮助 |
| `Esc` | 清除筛选、返回工具列表或关闭面板 |
| `q` / `Ctrl+C` | 退出；弹窗中的 `q` 先关闭弹窗 |

写入确认默认选择“取消”；按 `Tab` 选择确认后回车，或按 `y` / `Y` 执行。确认期间供应商或可见状态发生变化时，会要求重新确认。并存工具的客户端配置无法读取时，TUI 显示“状态未知”并阻止启用 / 停用。

浏览、搜索、刷新和退出不会写配置，也不会显示密钥或令牌。TUI 与 App、命令行共用目录覆盖和写入保护，支持 `--home` 隔离运行；刷新使用 `r`，不会自动监控文件。正在执行的写入会先完成再响应中断。TUI 需要真实终端，不支持管道或 `--json`。

### 交互菜单

```sh
leanflip                   # 打开工具菜单
leanflip menu codex        # 直接进入 Codex 供应商菜单
leanflip --lang en menu    # 使用英文菜单
```

| 按键 / 操作 | 效果 |
| --- | --- |
| `↑` / `↓` | 移动选择 |
| 直接输入文字 | 筛选当前菜单项 |
| `Enter` | 选择或进入操作确认 |
| `Esc` | 返回上一层或退出 |
| `Ctrl+C` | 退出菜单并恢复终端光标 |

菜单会标出“当前使用”“已启用”“未启用”或“状态未知”，并提供“导入当前客户端配置”和“查看配置文件路径”。执行写入前会显示完整目标及配置路径；取消确认不会执行该操作。

交互菜单要求输入、输出和错误输出都连接到真实终端，且 `TERM` 不是 `dumb`。在管道、重定向或脚本中，请使用命令模式。`--json` 不会启动菜单。

### 常用命令

```sh
leanflip status                          # 查看全部工具概览
leanflip list codex                      # 查看 Codex 的供应商和真实 ID
leanflip paths codex                     # 查看实际配置路径
leanflip use codex PROVIDER_ID           # 切换 Codex 供应商
leanflip enable opencode PROVIDER_ID     # 写入一个并存式供应商
leanflip disable opencode PROVIDER_ID    # 移出客户端配置，保留供应商
leanflip import live claude              # 从当前 Claude Code 配置导入
leanflip --help                          # 总帮助
leanflip provider --help                 # 供应商管理帮助
leanflip import file --help              # 文件导入帮助
leanflip --lang en status                # 英文输出
```

通常不需要用 `sudo` 运行 `leanflip`；使用其他用户身份会改变默认 HOME 和配置位置。

### 新增和更新供应商

常用连接设置可以直接交互编辑，不需要编写 JSON：

```sh
leanflip provider edit claude PROVIDER_ID
leanflip provider edit codex PROVIDER_ID
```

交互菜单也提供“编辑供应商 · Base URL / API Key”；TUI 选择供应商后按 `e`。现有 API Key 不会填入输入框，留空保留原值；需要替换时隐藏输入。TUI 用 `Tab` 切换字段、左右键移动光标、`Ctrl+U` 清空当前字段，`Ctrl+K` 明确清空密钥，`Ctrl+S` 检查保存，`Esc` 取消或确认放弃。确认页默认选择取消。修改正在使用的供应商会同步客户端配置，未使用的供应商不会因此启用。

列表及详情显示 Base URL 和密钥配置状态，敏感的 URL 部分会隐藏；官方登录及云认证等高级配置仍在 App 中管理。供应商被其他进程修改，或编辑期间配置目录发生变化时，需要重新打开编辑并检查。需要修改完整配置时，使用下面的 JSON 命令。

以 Claude Code 为例，把下面的内容保存为私有文件 `provider.json`，在本地编辑器中替换示例地址和密钥：

```json
{
  "id": "my-claude",
  "name": "我的 Claude 供应商",
  "settingsConfig": {
    "env": {
      "ANTHROPIC_BASE_URL": "https://api.example.com",
      "ANTHROPIC_AUTH_TOKEN": "replace-with-your-api-key"
    }
  }
}
```

示例地址不能用于真实请求；请使用供应商给出的、与该客户端兼容的地址。文件中包含密钥时，可先执行 `chmod 600 provider.json` 限制读取权限。

```sh
leanflip provider add claude --file provider.json     # 只保存
leanflip list claude                                  # 确认 ID
leanflip use claude my-claude                         # 需要使用时再切换
```

首次添加时也可以加 `--activate`，一次完成保存和应用；同一 ID 已存在时应使用 `update`，不要重复 `add`。

```sh
leanflip provider update claude --file provider.json  # 按文件里的 ID 更新
cat provider.json | leanflip provider update claude   # 也支持标准输入
leanflip provider delete claude PROVIDER_ID           # 删除前先确认它没有在使用
```

`update` 接收完整的供应商定义，不是局部 JSON 补丁；请保留需要的字段。不同工具的 `settingsConfig` 结构不同，例如 Codex 使用 `{ "auth": { ... }, "config": "TOML 原文" }`。可以先在 App 中使用对应工具的预设，或导入已有配置作为参考。

输入上限为 1 MiB。请通过文件或标准输入提供密钥，避免把真实密钥放在命令参数、Shell 历史或公开问题反馈中。

### JSON 输出与脚本

```sh
leanflip status --json
leanflip list codex --json
leanflip paths --json
```

成功结果写入标准输出，基本结构为 `{"ok":true,"data":...}`；失败结果写入标准错误，结构为 `{"ok":false,"error":{"code":"...","message":"..."}}`。`data` 随命令变化，脚本应依据退出码和稳定错误码处理结果。

| 退出码 | 含义 |
| --- | --- |
| `0` | 操作成功，或正常显示帮助 |
| `1` | 读取、写入、网络等执行失败 |
| `2` | 参数、输入或操作前置条件不正确，例如工具模式不匹配、试图删除正在使用的供应商 |
| `130` | 交互菜单或 TUI 被 `Ctrl+C` 中断 |

`--lang` 只改变人读输出和帮助，不改变 JSON 字段、英文错误消息或退出码。列表与状态输出不包含供应商配置原文或登录令牌。并存式工具读取失败时，`enabled` / `inLive` 可能为 `null`，应按“未知”处理。

`--json` 后仍需给出具体命令；设备码登录会额外在标准错误输出 `device_login` 事件。普通文本输出支持 `head` 等提前关闭管道的用法；JSON 输出管道提前关闭按执行失败处理。更多细节见 [CLI 完整说明](crates/cli/README.md)。

### 在隔离目录试用

如果只想先熟悉命令，可以指定一个全新的临时 HOME：

```sh
LEANFLIP_DEMO_HOME=$(mktemp -d)
leanflip --home "$LEANFLIP_DEMO_HOME" status
leanflip --home "$LEANFLIP_DEMO_HOME" menu
```

后续每条试用命令都要带上同一个 `--home`。它使用该目录的默认布局，忽略 `CODEX_HOME` 等工具环境变量，但仍会应用该目录内已有的 App 目录覆盖设置，因此完整隔离应从空目录开始。真实账号登录会访问外部服务，试用菜单本身不需要登录。

## 从 CC Switch 迁移

LeanFlip 可以只读导入 CC Switch 的数据。CC Switch 通常使用 `~/.cc-switch/`，LeanFlip 的数据保存在 `~/.leanflip/`，两者互不干扰。迁移不会修改 CC Switch 的任何文件，也不会自动切换当前供应商。

| 数据来源 | App 入口 | 命令行入口 | 导入内容 |
| --- | --- | --- | --- |
| 某个 AI 工具当前的配置 | 工具页 → 导入当前配置 | `leanflip import live TOOL` | 从客户端配置生成供应商 |
| 本机 CC Switch 数据目录 | 数据与迁移 → 本机数据目录 | `leanflip import local`，TUI 按 `b` | 支持的供应商；选中 Codex 时可一并导入兼容的托管账号 |
| CC Switch 导出文件 | 数据与迁移 → 导入文件 | `leanflip import file FILE` | 支持的供应商，不导入 OAuth 登录凭据 |

### 本机数据目录

```sh
leanflip import local                              # 自动检测，只预览
leanflip import local --accounts --apply           # 确认迁入供应商和托管账号
leanflip import local --dir /path/to/.cc-switch    # 手动指定绝对路径
leanflip import local --apply --tool claude,codex  # 仅迁入所选工具，不导入账号
leanflip tui                                       # 按 b → 检测本机 CC Switch
```

命令行只有显式指定 `--accounts` 才迁入登录凭据；App 和 TUI 的预览中可以取消账号勾选。迁入托管账号时需要选中 Codex。

1. 打开 App 左侧“数据与迁移”，选择“本机数据目录”。
2. 点击“检测本机 CC Switch”，自动识别默认位置及 CC Switch 保存的自定义数据目录；不需要启动 CC Switch。
3. 检查识别到的目录、供应商和账号。检测不到时展开“手动指定数据目录”，填写绝对路径再扫描。
4. 选择工具并确认导入；更换来源目录后需要重新扫描。
5. 到对应工具页面确认导入结果，再决定是否切换或启用。

支持读取 `cc-switch.db` 或旧版 `config.json`。已存在的供应商不会被覆盖；需要本地代理、路由或专用登录的部分条目会被跳过，预览会显示原因。

### 导出文件

支持 `.sql` SQL 导出、`.db` SQLite 快照和旧版 `.json` 配置。命令行默认只预览，明确加 `--apply` 才写入：

```sh
leanflip import file backup.sql --preview
leanflip import file backup.sql --apply --tool claude,codex
leanflip import file backup.sql --apply --duplicate rename --tool codex
```

`--tool` 可以重复指定，也可以用逗号分隔；省略时处理支持的全部工具。

- `--duplicate skip`：默认策略，跳过已存在的同 ID 供应商。
- `--duplicate rename`：同 ID 且配置不同时，改名后导入；配置相同的仍跳过。

文件导入不会切换到新供应商，不导入 OAuth 登录凭据，并会清理推广链接。它用于迁移供应商，不等同于完整的账号备份恢复；在 LeanFlip 设备之间完整迁移，请使用下面的加密备份。

## 加密完整备份与恢复

`.leanflip` 是三端共用的完整迁移格式，使用标准 **age 密码加密（scrypt）**，不会有明文备份文件落盘。早期版本导出的 `.switchleaf` 备份可以直接导入。导出时输入两次 10–4096 个字符的密码；密码不会保存，遗失后无法恢复。

| 内容 | 备份 | 恢复行为 |
| --- | --- | --- |
| 供应商、配置和 API Key | 包含 | 按工具选择；默认跳过重复 ID，可另存冲突副本 |
| 托管 Codex 账号及登录凭据 | 包含 | 可选；已有相同账号时保留本机凭据，不覆盖较新的登录 |
| 默认托管账号 | 包含 | 遵循账号合并规则，保留已有默认账号 |
| 语言、外观、关闭到托盘、后台刷新等偏好 | 包含 | 可选；同时包含保留 Codex 官方登录的切换偏好 |
| 工具目录覆盖 | 包含 | 默认不恢复；确认目标设备具有相同路径布局后再勾选 |
| 开机启动 | 记录 | 保留目标设备的设置，需要在目标 App 中单独开启 |
| 客户端当前配置、当前供应商 | 不应用 | 恢复后在工具页或 TUI 中单独切换 |
| 独立客户端登录暂存、客户端会话、日志、历史快照 | 不迁移 | 客户端登录需要在目标设备重新认证；可按下节进行本机目录备份 |

**App：**“数据与迁移 → 导出备份”，设置密码，选择保存位置。恢复时在“导入文件”中选择 `.leanflip`，输入密码解锁，查看摘要并勾选恢复范围，再点击“确认恢复所选内容”。

**TUI：**`leanflip tui` 后按 `b`。方向键选择“导出加密备份”或“导入文件”；`Tab` 切换字段，密码显示为圆点。预览中用 `↑` / `↓` 选择、`Space` 勾选、`d` 切换重复策略；`Enter` 打开最终确认，默认选中取消，`y` 执行；`Esc` 取消或返回。文件路径支持 `~/`，不需要加引号。

**命令行：**默认隐藏输入密码，导入默认只预览，加 `--apply` 才执行：

```sh
leanflip export ~/Backups/workspace.leanflip
leanflip import backup ~/Backups/workspace.leanflip
leanflip import backup ~/Backups/workspace.leanflip --apply

# 只恢复 Claude 的供应商，保留本机账号与设置
leanflip import backup workspace.leanflip --tool claude --no-accounts --no-settings --apply
# 只恢复托管账号
leanflip import backup workspace.leanflip --no-providers --no-settings --apply
# 冲突的供应商另存为副本；路径仍默认保留本机设置
leanflip import backup workspace.leanflip --duplicate rename --apply
```

脚本模式和 `--json` 使用 `--password-file /private/path/passphrase.txt`，从私有 UTF-8 文件读取口令（忽略一个结尾换行）。不要把密码写在命令参数或 Shell 历史中；密码文件建议权限 `0600`。导出目录需要已存在，已有文件不会被覆盖；加密备份上限 32 MiB，在 Unix 下导出文件权限为 `0600`。

错误密码、损坏文件或单纯预览都不会恢复数据。恢复采用合并，不会清空现有数据；后续阶段发生磁盘错误时，界面会报告部分完成并保留已经写入的数据，修复后可以重试。恢复不会调用登录服务，也不会自动切换客户端；恢复的登录凭据是否仍然有效，需要由之后的客户端使用或账号刷新确认。

解密后的预览在 10 分钟后自动清除，恢复前还会再次检查有效期；离开备份页或关闭 App 窗口也会丢弃预览。

## Codex 登录与多账号

### 使用本机已有的 ChatGPT 登录

Codex 官方供应商可以选择“本机的 ChatGPT 登录（自动）”。切换到它时使用本机 `auth.json` 或登录暂存中的登录；第三方供应商通常使用自己的 API 配置。

“设置 → 切到第三方时保留 Codex 的 ChatGPT 登录”默认关闭：切到第三方时会暂存本机登录并从 `auth.json` 移出，切回采用本机登录的官方供应商时恢复。开启后可以保留登录，实际请求行为仍由客户端配置决定。

这里的登录操作针对 `leanflip paths codex` 显示的配置目录；使用前请确认它就是要管理的 Codex 客户端目录。

### 托管多个 ChatGPT 账号

App 中的流程：

1. 打开“Codex 账号”，点击“登录新账号”。
2. 按界面给出的登录网址和短验证码，在浏览器中完成确认。
3. 等待账号出现在列表中。
4. 回到 Codex 工具页，添加或编辑一个官方供应商，在“登录方式”中选择指定托管账号或“默认账号”。
5. 保存并按需切换到它；编辑正在使用的供应商时，保存会更新客户端配置。

命令行也可以管理账号：

```sh
leanflip accounts list
leanflip accounts login
leanflip accounts login --timeout 600
leanflip accounts default ACCOUNT_ID
leanflip accounts login --account ACCOUNT_ID
leanflip accounts refresh ACCOUNT_ID
leanflip accounts remove ACCOUNT_ID
```

`ACCOUNT_ID` 替换为 `accounts list` 中的 ID。登录默认等待 300 秒，`--timeout` 可设为 1–1800 秒；超时后重新执行会生成新的登录流程。

“设为默认”只更新托管账号库，供绑定默认账号的供应商下次切换时使用；它本身不会立即改动客户端。命令行完成登录后，仍需切换到绑定该账号的官方供应商。

账号令牌保存在本机，界面和命令行的账号列表不会输出令牌。移除托管账号不会在 OpenAI 服务端注销或撤销该账号，绑定它的供应商需要重新指定可用的登录。后台定期刷新由运行中的 App 提供，命令行没有常驻刷新服务。

## 配置目录与数据备份

### 查看实际生效路径

以 App 工具页顶部显示的配置文件或命令行输出为准：

```sh
leanflip paths
leanflip paths codex
```

常见默认路径如下。设置中的目录覆盖、相关环境变量和旧版文件兼容规则可能改变实际位置。

| 工具 | 默认配置位置 |
| --- | --- |
| Claude Code | `~/.claude/settings.json`；只有旧版 `claude.json` 时继续使用旧文件 |
| Codex | `~/.codex/config.toml`、`~/.codex/auth.json` |
| Gemini CLI | `~/.gemini/.env`、`~/.gemini/settings.json` |
| Grok Build | `~/.grok/config.toml` |
| Claude Desktop | macOS 的 `~/Library/Application Support/Claude/` 和 `Claude-3p/`；Linux 的对应配置根目录下同名目录 |
| OpenCode | `~/.config/opencode/opencode.jsonc`，已有该文件时优先；否则 `opencode.json` |
| OpenClaw | `~/.openclaw/openclaw.json` |
| Hermes | `~/.hermes/config.yaml` |
| Pi | `~/.pi/agent/models.json` |
| MCode | `~/.minimax/config.yaml` |

普通运行会遵循 `CODEX_HOME`、`HERMES_HOME`、`PI_CODING_AGENT_DIR`、`MINIMAX_DATA_DIR` / `MAVIS_DATA_DIR` 等已支持的路径环境变量，再应用 App 保存的目录覆盖。自定义配置目录时，建议在 App 中设置后运行 `leanflip paths` 核对。

写入含凭据的客户端文件时，Unix 权限统一收紧到 `0600`；新建文件同样使用 `0600`。Gemini 的无凭据 `settings.json` 保留已有权限。原子写入会跟随文件及目录符号链接，在真实目标目录中暂存并替换目标，保留链接本身。

Claude Code 的供应商只管理连接、鉴权、模型及已支持的供应商参数；`permissions`、`hooks`、`env.API_TIMEOUT_MS`、`env.DISABLE_TELEMETRY` 等字段保留客户端原值。保存、切换和导入预览会提示被忽略的字段名，不显示字段值。

### LeanFlip 保存哪些数据

默认在 `~/.leanflip/`：

| 文件 / 目录 | 用途 |
| --- | --- |
| `providers.json` | 供应商、当前选择和相关切换设置 |
| `app-settings.json` | 语言、外观、目录覆盖、托盘及后台刷新等设置 |
| `codex_oauth_auth.json` | 托管 Codex 账号及登录凭据 |
| `codex-login-stash.json` | 切换时暂存的本机 Codex 登录 |
| `codex_managed_oauth_live_auth.json` | 当前托管登录与客户端的对应标记 |
| `live-state.json` | 已完成及尚未完成的写入状态 |
| `backups/live-first-write/` | 每个客户端文件首次写入前的原始备份及来源路径记录 |
| `usage.db` | 每日 token 用量汇总、切换记录和各会话记录的读取进度；不含对话内容 |

从旧版本升级时，原来的数据目录会在 App 启动或命令行第一次执行写操作时整体移动到 `~/.leanflip/`，并在原位置留下一个指向新目录的链接；只读命令在迁移前继续读取原目录。

数据文件可能包含 API Key 或登录凭据，并不是加密保险箱。请把它们作为私有数据保存，分享截图、日志或提交 Issue 前移除敏感内容。

### 手动备份

先从托盘退出 App，并结束正在写配置的命令。下面的命令把数据目录备份到一个带时间戳的目录：

```sh
(
  umask 077
  LEANFLIP_BACKUP_DIR="$HOME/leanflip-backup-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$LEANFLIP_BACKUP_DIR"
  cp -a "$HOME/.leanflip" "$LEANFLIP_BACKUP_DIR/"
)
```

如需恢复“客户端当前正在使用的完整配置”，还应另外备份 `leanflip paths` 显示的客户端文件。首次写入备份只保存每个路径第一次写入前的状态，不是每次切换的历史版本。

备份目录中的 `.source` 文件记录原始路径；路径经符号链接解析后不同时，另有 `.resolved` 文件记录首写时的真实目标。原文件当时不存在时，可能只有来源记录而没有原始文件备份。手工恢复前，请先退出相关客户端并保存当前文件，再核对路径和备份内容。

### 中断恢复

App 启动和相关写操作会处理未完成的配置写入。命令行的 `status`、`list`、`paths` 和文件导入预览保持只读，不会自动执行恢复；需要显式处理时使用：

```sh
leanflip recover codex   # 只处理 Codex
leanflip recover         # 处理所有工具
```

`recover` 用于补完或清理中断的操作，**不表示撤销上一次切换**。如果恢复提示某些文件已被外部修改或只能部分恢复，请通过 `paths` 定位后检查；不要直接覆盖仍在使用的新配置或新登录凭据。

## 常见问题

### `leanflip: command not found`

确认已经把二进制放入 PATH，并重新打开终端。也可以直接运行 `./target/release/leanflip --version` 验证构建结果，参见[构建命令行与 TUI](#构建命令行与-tui)。

### `Permission denied`、`Exec format error` 或 `Bad CPU type`

确认二进制是在当前设备上构建的，或与当前系统和 CPU 架构一致。复制过程中丢失执行权限时，可执行 `chmod +x leanflip`。Linux 构建默认链接 glibc，不能假定可以直接用于 Alpine 等 musl 环境。

### 工具显示“未检测到配置”

这表示预期位置的配置文件还不存在，不代表已经安装了那个 AI 工具。先运行 `leanflip paths TOOL` 核对位置；自定义目录在设置中填写。必要时先在对应客户端完成首次启动或登录。

### 导入成功了，为什么配置没切换？

从 CC Switch 迁移只保存供应商。到工具页点击“切换”或打开开关，或执行 `leanflip use TOOL PROVIDER_ID` / `leanflip enable TOOL PROVIDER_ID`。新增的供应商默认也只保存，除非明确选择立即应用。

### “导入当前配置”显示没有新条目

切换式工具只在还没有任何供应商时进行首次导入；已经有供应商时会保持原样。并存式工具只补齐缺少的条目。要修改已有供应商，请使用编辑器或 `provider update`。

### `invalid_tool_mode`，或菜单无法打开

切换式工具使用 `use`，并存式工具使用 `enable` / `disable`。菜单需要交互终端；管道、重定向、`TERM=dumb` 或 `--json` 场景请改用具体命令。

### 提示配置读取失败、状态未知、写入冲突或权限错误

先运行 `leanflip paths TOOL` 找到实际文件，检查其 JSON / TOML / YAML 格式、写入权限，以及是否有其他程序同时修改。修复后重试读取。App 和命令行的跨进程锁用于协调 LeanFlip 自己的操作，不能阻止其他编辑器或客户端改写文件。

单次锁获取最多等待 10 秒，超时返回 `lock.busy`。App 会显示锁路径及最近记录的 PID；命令行使用固定的错误码和文案。先等待另一个操作结束，或检查对应进程是否被暂停，再重试。锁文件中的 PID 可能属于已经退出的上一个持有者，不能据此自动终止进程。

### 显示已切换，但客户端仍使用原供应商

重新加载客户端配置或开启新会话，并检查是否使用了其他配置目录、进程环境变量或启动参数。“使用中”是 LeanFlip 保存的状态；LeanFlip 只在你点击“测连通”“获取模型列表”“检查模型”时访问供应商，平时不会自动检测密钥、余额或模型可用性。

### Codex 提示需要重新登录，或出现 `codex.oauth.live_sync_failed`

“需要重新登录”时，在 App 账号页重新登录，或执行 `leanflip accounts login --account ACCOUNT_ID`。

`codex.oauth.live_sync_failed` 表示新凭据已保存到账号库，但同步客户端失败。检查 Codex 配置路径和权限后，再切换到对应的官方供应商；不要用旧令牌覆盖已保存的新凭据。

### 关闭 App 窗口后为什么还在运行？

默认开启“关闭窗口时留在托盘”。可以从托盘选择退出，或在设置中关闭此选项。命令行不需要桌面 App 常驻。

### 如何反馈问题？

在 [Issues](https://github.com/Ricardo121380/LeanFlip/issues) 中提供：App 或命令行版本、操作系统与架构、使用的工具、复现步骤、预期结果和实际错误码。路径可以脱敏；请勿附上真实 API Key、OAuth 令牌或完整账号库。

## 升级与卸载

从源码升级时，先退出 App 和交互菜单，然后拉取最新代码并重新构建：

```sh
git pull
cargo build --release --locked --bin leanflip
```

桌面 App 按[构建桌面 App](#构建桌面-app)重新打包后替换原来的 App。升级前建议备份 LeanFlip 数据和客户端配置。

卸载 App 或命令行不会自动还原已经写入客户端的配置。若希望回到官方或其他供应商，请在卸载前先完成切换。

- 命令行：删除自己安装的 `leanflip` 二进制，例如 `~/.local/bin/leanflip`。
- macOS App：从“应用程序”中移除 LeanFlip。
- Linux DEB：使用 `apt` 移除安装的 LeanFlip 软件包。

`~/.leanflip/` 中的供应商、账号和备份由用户自行保留或清理。删除前请确认不再需要其中的登录暂存和备份。

## 开发

在仓库根目录安装依赖并运行检查：

```sh
pnpm install --frozen-lockfile --ignore-scripts
pnpm typecheck
pnpm test
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

**`pnpm tauri dev` 会使用当前用户的真实配置。** 只想调整界面时，用内置的演示后端在浏览器里打开前端，不会读写任何真实文件：

```sh
pnpm dev:mock   # 打开 http://localhost:1420
```

需要在真实 App 中试用时，用项目提供的假配置填充一个临时目录，再通过 `LEANFLIP_HOME` 指向它启动开发版。启动前先退出正在运行的 LeanFlip，否则单实例保护会把窗口切到已运行的 App：

```sh
LEANFLIP_DEMO_HOME=$(mktemp -d)
cargo run --locked --example seed_demo -- "$LEANFLIP_DEMO_HOME"
env -u CODEX_HOME -u HERMES_HOME -u PI_CODING_AGENT_DIR \
  -u MINIMAX_DATA_DIR -u MAVIS_DATA_DIR \
  LEANFLIP_HOME="$LEANFLIP_DEMO_HOME" \
  XDG_CONFIG_HOME="$LEANFLIP_DEMO_HOME/.config" \
  pnpm tauri dev
```

演示目录包含假的供应商和账号，不能当作可用的在线服务凭据。

终端界面的伪终端检查需要先构建命令行，只依赖 Python 3 标准库，使用临时 HOME：

```sh
python3 scripts/check-cli-menu.py --binary target/release/leanflip
python3 scripts/check-cli-tui.py --binary target/release/leanflip
```

- [检查工作流](.github/workflows/ci.yml)：主分支 push 和 PR 时运行类型检查、前端测试、Rust 测试、Clippy，以及 Linux / macOS 桌面打包验证；命令行单独构建、检查并上传压缩包。
- [发布工作流](.github/workflows/release.yml)：版本标签触发正式 App / 命令行构建，校验版本与架构，生成 SHA256 文件并创建草稿 Release。
- [分发维护说明](docs/distribution.md)：Homebrew、npm 安装器和手动发布流程。

内置预设由 LeanFlip 自己维护，覆盖全部 10 种工具：

```sh
pnpm presets
```

`api.example.com` 和 `your-model-id` 是需要替换的示例值，Grok 的 `context_window` 也应按所选模型调整。

## 项目结构与许可证

| 路径 | 内容 |
| --- | --- |
| [`crates/core`](crates/core) | 配置投影与写入、供应商库、首次写入备份、中断恢复、导入和 Codex OAuth；不依赖 Tauri |
| [`crates/service`](crates/service) | App 与命令行共用的业务层、设置、路径覆盖、脱敏及 OAuth HTTP 实现 |
| [`crates/app`](crates/app) | Tauri 2 桌面外壳、托盘、文件监听及后台账号刷新 |
| [`crates/cli`](crates/cli) | 命令行 `leanflip`、交互菜单及全屏 TUI |
| [`ui`](ui) | React、TypeScript、Vite、TanStack Query 前端及中英文界面 |
| [`scripts`](scripts) | 预设生成、终端伪终端检查及分发元数据生成脚本 |
| [`packages/npm`](packages/npm) | npm 安装器；Homebrew Cask 与安装器元数据在发布时生成 |

当前源码以 [MIT License](LICENSE) 发布。实现依据、兼容边界及核查记录见[独立实现记录](docs/engineering/independent-core.md)，保留的历史标识及原因见[兼容说明](docs/engineering/compatibility.md)。早期 v0.1.0 发布包适用其随附的许可证与通知。第三方依赖适用各自的许可证。
