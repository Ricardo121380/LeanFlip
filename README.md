# CC Switch Lite

**用桌面 App 或终端菜单，管理 AI 编码工具的供应商配置。**

CC Switch Lite 将供应商保存为本地卡片，按需写入 Claude Code、Codex、Gemini CLI 等工具的配置文件。App 与独立 CLI `ccsl` 共用同一份数据：可以先在 App 中添加供应商，再在终端用方向键切换。

项目专注于配置管理，不提供本地代理、请求转发、负载均衡或用量统计。使用前需要自行安装对应的 AI 工具，并准备该工具支持的账号或 API Key。

[下载 v0.1.0](https://github.com/Ricardo121380/ccswitch-lite/releases/tag/v0.1.0) · [查看最新发布](https://github.com/Ricardo121380/ccswitch-lite/releases/latest) · [CLI 完整说明](crates/cli/README.md) · [反馈问题](https://github.com/Ricardo121380/ccswitch-lite/issues)

## 目录

- [选择 App 还是 CLI](#选择-app-还是-cli)
- [下载与安装](#下载与安装)
- [第一次使用](#第一次使用)
- [支持的工具与操作方式](#支持的工具与操作方式)
- [桌面 App 使用指南](#桌面-app-使用指南)
- [CLI 使用指南](#cli-使用指南)
- [从 cc-switch 迁移](#从-cc-switch-迁移)
- [Codex 登录与多账号](#codex-登录与多账号)
- [配置目录与数据备份](#配置目录与数据备份)
- [常见问题](#常见问题)
- [升级与卸载](#升级与卸载)
- [从源码构建与开发](#从源码构建与开发)
- [项目结构与许可证](#项目结构与许可证)

## 选择 App 还是 CLI

| 使用场景 | 建议入口 |
| --- | --- |
| 第一次配置，希望从预设填写地址、密钥和模型 | 桌面 App |
| 希望用方向键选择工具、搜索供应商、回车执行 | CLI 交互菜单：`ccsl` |
| 通过 SSH 使用，或在没有桌面环境的机器上管理配置 | 独立 CLI |
| 在脚本中查询状态或切换供应商 | CLI 命令模式，配合 `--json` |
| 从本机 cc-switch 数据目录迁移卡片及托管账号 | App 的“从 cc-switch 导入” |

App 和 CLI 可以单独安装，也可以一起使用。运行预编译的 CLI 不需要 Node.js、Rust、WebView 或桌面 App。

## 下载与安装

### 通过 Homebrew 或 npm 安装

**Homebrew：**先将本仓库添加为软件源，然后按需安装 App 或 CLI：

```sh
brew tap ricardo121380/ccswitch-lite https://github.com/Ricardo121380/ccswitch-lite.git

# 桌面 App：macOS Apple Silicon
brew install --cask ricardo121380/ccswitch-lite/ccswitch-lite

# CLI：macOS Apple Silicon / Linux x86_64
brew install --cask ricardo121380/ccswitch-lite/ccsl
ccsl
```

两个入口都使用二进制 Cask，直接安装发布包、校验 SHA256，并管理安装与升级；无需本机编译。Linux CLI Cask 需要支持 Linux Cask 的 Homebrew 7 或更新版本。桌面 Cask 当前支持 macOS ARM64；Linux 桌面端使用下面的 DEB 安装方式或 npm 桌面安装器。

macOS 的 App 和 CLI 当前都未做 Apple Developer ID 签名及公证。Homebrew 安装成功后，首次运行仍可能被系统拦截，包括 `ccsl --version`；请按下文的[首次启动说明](#macos-提示无法验证开发者或文件已损坏)手动允许。

```sh
brew update
brew upgrade --cask ricardo121380/ccswitch-lite/ccsl
brew upgrade --cask ricardo121380/ccswitch-lite/ccswitch-lite
```

**npm：**安装器已准备好，目前尚未发布到 npm 官方仓库，不能直接执行 `npm install -g ccswitch-lite`。可以从仓库生成并安装本地 npm 包，无需编译 Rust 或安装前端依赖：

```sh
git clone https://github.com/Ricardo121380/ccswitch-lite.git
cd ccswitch-lite
mkdir -p target/npm
CCSL_NPM_TARBALL=$(npm pack ./packages/npm --ignore-scripts --pack-destination target/npm --silent)
npm install -g "./target/npm/$CCSL_NPM_TARBALL" --ignore-scripts

ccsl                         # 原生 CLI 菜单
ccsl-app install             # 安装桌面 App
ccsl-app open                # 启动桌面 App
```

npm 方式需要 Node.js 20 或更新版本。一个包提供两个命令，安装时不执行生命周期脚本；首次使用时从 GitHub 下载固定版本的原生程序并校验 SHA256。CLI 与直接下载的二进制具有相同的命令、JSON 输出和交互菜单。

macOS 的 `ccsl-app install` 默认将 App 安装到 `~/Applications`；Linux 使用 `apt` 安装 DEB，普通用户需在终端中通过 `sudo` 确认。更新 App 前先退出应用，再运行 `ccsl-app install --replace`。也可先用 `ccsl-app download --output ./downloads` 仅下载并校验安装包。完整参数见 [npm 安装器说明](packages/npm/README.md)。

CLI 选择一种包管理方式即可；如果同时安装了多份，可用 `command -v ccsl` 确认正在使用的入口。

### 1. 选择对应的安装包

下面的链接对应 **v0.1.0**。GitHub 页面里的 `Source code (zip)` / `Source code (tar.gz)` 是源码，需要自行构建；直接使用请选择下表中的附件。

| 平台 | 桌面 App | 独立 CLI |
| --- | --- | --- |
| macOS Apple Silicon，M 系列芯片 | [DMG 安装包](https://github.com/Ricardo121380/ccswitch-lite/releases/download/v0.1.0/ccswitch-lite_0.1.0_macos-arm64.dmg) | [CLI 压缩包](https://github.com/Ricardo121380/ccswitch-lite/releases/download/v0.1.0/ccsl_0.1.0_macos-arm64.tar.gz) |
| Linux x86_64 / amd64 | [DEB 安装包](https://github.com/Ricardo121380/ccswitch-lite/releases/download/v0.1.0/ccswitch-lite_0.1.0_linux-x64.deb) | [CLI 压缩包](https://github.com/Ricardo121380/ccswitch-lite/releases/download/v0.1.0/ccsl_0.1.0_linux-x64.tar.gz) |

Linux 安装包在 Ubuntu 24.04 上构建和验证，CLI 使用 glibc；其他发行版需自行验证。首版未提供 Windows、Intel Mac 或 Linux ARM 的预编译安装包。

不确定处理器类型时，可在终端运行 `uname -m`：Apple Silicon 通常显示 `arm64`，Linux x86_64 显示 `x86_64`。

### 2. 安装桌面 App

**macOS：**

1. 打开下载的 `ccswitch-lite_0.1.0_macos-arm64.dmg`。
2. 将 **CC Switch Lite.app** 拖入“应用程序”。
3. 从“应用程序”启动。

本版尚未配置 Apple Developer ID 签名和公证。如果提示无法验证开发者，先确认文件来自本仓库、校验值一致，再按系统提示进入“系统设置 → 隐私与安全性 → 仍要打开”。具体步骤见 [Apple 官方说明](https://support.apple.com/zh-cn/102445)。

**Ubuntu / Debian 系：**

在下载目录打开终端，运行：

```sh
sudo apt install ./ccswitch-lite_0.1.0_linux-x64.deb
```

安装后，从应用菜单启动 **CC Switch Lite**。`apt` 会处理安装包声明的系统依赖；桌面 App 需要可用的图形桌面环境。

### 3. 安装独立 CLI

在下载目录中，执行对应平台的解压命令。

macOS Apple Silicon：

```sh
mkdir -p ccsl-0.1.0
tar -xzf ccsl_0.1.0_macos-arm64.tar.gz -C ccsl-0.1.0
./ccsl-0.1.0/ccsl --version
./ccsl-0.1.0/ccsl
```

Linux x86_64：

```sh
mkdir -p ccsl-0.1.0
tar -xzf ccsl_0.1.0_linux-x64.tar.gz -C ccsl-0.1.0
./ccsl-0.1.0/ccsl --version
./ccsl-0.1.0/ccsl
```

`--version` 应显示 `ccsl 0.1.0`。在普通交互终端中，不带参数启动就会打开菜单。

希望在任何目录直接输入 `ccsl`，可以安装到当前用户的命令目录：

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 ccsl-0.1.0/ccsl "$HOME/.local/bin/ccsl"
export PATH="$HOME/.local/bin:$PATH"
ccsl --version
```

上面的 `export` 对当前终端生效。要让新终端也能找到命令，将 `export PATH="$HOME/.local/bin:$PATH"` 加入所用 Shell 的启动文件，例如 zsh 的 `~/.zshrc` 或交互式 bash 的 `~/.bashrc`，然后重新打开终端。

下文的 `ccsl` 假定已加入 PATH；如果只解压使用，请改为实际二进制路径，例如 `./ccsl-0.1.0/ccsl`。

### 4. 核对下载文件

同一 Release 提供 [SHA256SUMS](https://github.com/Ricardo121380/ccswitch-lite/releases/download/v0.1.0/SHA256SUMS)。计算下载文件的 SHA256，与其中同名文件所在行比较：

```sh
# macOS 示例
shasum -a 256 ccsl_0.1.0_macos-arm64.tar.gz

# Linux 示例
sha256sum ccsl_0.1.0_linux-x64.tar.gz
```

检查 App 安装包时，将文件名替换为下载的 `.dmg` 或 `.deb`。校验值不同应重新下载，确认一致后再使用。

## 第一次使用

### 已经在使用某个 AI 工具

1. 在 App 左侧选择对应工具，例如 **Claude Code** 或 **Codex**。
2. 展开“配置文件”，确认读取的是你实际使用的目录；自定义目录可在“设置 → 配置目录”中填写。
3. 点击“从当前配置导入”，把客户端现有配置保存为卡片。
4. 根据需要添加其他供应商，然后点击“切换”或启用开关。
5. 回到对应 AI 工具，重新打开会话或重新加载配置，确认实际使用的供应商和模型。

CLI 中对应的入门流程：

```sh
ccsl paths codex          # 先确认配置路径
ccsl import live codex    # 导入当前客户端配置
ccsl menu codex           # 在菜单中选择供应商
```

“从当前配置导入”主要用于初次接管：**切换式工具已有卡片时，不会重新导入或覆盖这些卡片**；并存式工具可重复导入，补齐尚未保存的条目。导入动作本身不切换供应商；若上次有中断的写入，写操作入口会先处理恢复，详见[中断恢复](#中断恢复)。

### 还没有供应商配置

1. 在 App 中选择工具，点击“添加供应商”。
2. 搜索并选择适合的预设，或选择“自定义”。
3. 填写名称、ID，以及该供应商要求的地址、API Key、模型等字段。预设提供配置结构，实际账号和可用模型由供应商提供。
4. 保存卡片。需要立即使用时，勾选“保存后立即切换到这张卡”或“保存后立即写入客户端配置”。
5. 以后可以直接运行 `ccsl`，在菜单中使用这张卡。

只使用 CLI 的用户，可以通过 JSON 文件[新增供应商卡片](#新增和更新供应商卡片)。交互菜单提供选择、导入和应用操作，完整配置编辑使用 App 或 `provider` 命令。

### 从 cc-switch 迁移

已有 cc-switch 卡片时，直接参考[迁移指南](#从-cc-switch-迁移)。迁移完成后，需要自行选择要应用的卡片。

## 支持的工具与操作方式

**供应商卡片**是本应用保存的一组配置；**客户端配置**是 AI 工具实际读取的文件。新增卡片默认只保存到库中，应用卡片后才会写入客户端。

| 工具 | CLI 工具名 | 模式 | 常用操作 |
| --- | --- | --- | --- |
| Claude Code | `claude` | 单一配置切换 | `ccsl use claude PROVIDER_ID` |
| Codex | `codex` | 单一配置切换 | `ccsl use codex PROVIDER_ID` |
| Gemini CLI | `gemini` | 单一配置切换 | `ccsl use gemini PROVIDER_ID` |
| Grok Build | `grokbuild` | 单一配置切换 | `ccsl use grokbuild PROVIDER_ID` |
| Claude Desktop | `claude-desktop` | 单一配置切换 | `ccsl use claude-desktop PROVIDER_ID` |
| OpenCode | `opencode` | 多供应商并存 | `ccsl enable opencode PROVIDER_ID` |
| OpenClaw | `openclaw` | 多供应商并存 | `ccsl enable openclaw PROVIDER_ID` |
| Hermes | `hermes` | 多供应商并存 | `ccsl enable hermes PROVIDER_ID` |
| Pi | `pi` | 多供应商并存 | `ccsl enable pi PROVIDER_ID` |
| MCode | `mcode` | 多供应商并存 | `ccsl enable mcode PROVIDER_ID` |

`PROVIDER_ID` 是占位符，请替换为 `ccsl list 工具名` 中显示的真实 ID。名称用于展示，ID 用于命令和配置条目定位；App 中创建后不能修改 ID。

- **切换式工具**：同一时间选择一张卡，使用 `use` 切换。
- **并存式工具**：多张卡可以同时写入客户端，使用 `enable` / `disable` 启用或停用。具体使用哪个模型，仍由相应客户端的模型选择决定。
- **Hermes**：App 还提供“设为默认模型”，与供应商条目的启用开关分开。
- **Claude Desktop**：支持直连配置；模型菜单要求 `claude-sonnet-*`、`claude-opus-*`、`claude-haiku-*` 等受支持的官方模型名，不提供代理模式或模型名映射。

## 桌面 App 使用指南

### 查看和切换供应商

左侧按“单一配置 · 切换使用”和“多供应商 · 可同时启用”分组。进入工具页面后，可以：

- 按名称、ID 或地址搜索供应商。
- 查看当前选择，或已写入客户端的供应商数量。
- 展开、复制实际配置文件路径。
- 点击“切换”，或使用卡片开关写入 / 移出并存式供应商。
- 在读取失败时重试；并存配置不可读时，界面会阻止不可靠的启用 / 停用操作。

“当前选择”表示本应用保存的选择，不是连通性检测结果，也不能证明配置文件未被其他程序改动。实际模型访问是否成功，需要在对应客户端确认。

### 添加、编辑与删除

添加时可以搜索预设，也可以输入自定义配置。Codex 编辑器分别展示 `auth.json` 和 `config.toml`；其他工具按各自配置结构填写。

- 新增卡片默认只保存；勾选立即应用后才会同时写入客户端。
- **编辑正在生效的卡片并保存，会同步更新客户端配置。**
- 关闭编辑器、更换预设或离开有未保存修改的设置页时，会询问是否放弃修改。
- 当前切换式卡片需要先切换到其他卡片，才能删除。
- 删除并存式卡片时，也会从客户端移除对应条目；如果只是暂时停用，关闭开关即可保留卡片。

### 设置与托盘

| 设置 | 作用 |
| --- | --- |
| 界面语言 | 中文、英文或跟随系统；CLI 使用自己的 `--lang` 参数 |
| 配置目录 | 覆盖对应客户端目录，只接受绝对路径；留空使用默认值，保存后生效 |
| 开机自动启动 | 登录系统时启动 App；默认关闭 |
| 关闭窗口时留在托盘 | 默认开启；关闭窗口后仍可通过托盘切换供应商 |
| 托管账号后台刷新 | 账号上次取得令牌超过设定小时数后进入刷新范围；默认 168 小时，`0` 关闭 |
| 切到第三方时保留 Codex 的 ChatGPT 登录 | 控制第三方配置与本机 ChatGPT 登录的保留方式，详见[Codex 登录与多账号](#codex-登录与多账号) |

保存设置后，目录覆盖会被 App 和后续 CLI 命令共同使用。托盘菜单中可打开主窗口、切换供应商或“退出”应用；退出 App 后，其后台刷新任务也会停止。

## CLI 使用指南

### 交互菜单

```sh
ccsl                   # 打开工具菜单
ccsl menu codex        # 直接进入 Codex 供应商菜单
ccsl --lang en menu    # 使用英文菜单
```

| 按键 / 操作 | 效果 |
| --- | --- |
| `↑` / `↓` | 移动选择 |
| 直接输入文字 | 筛选当前菜单项 |
| `Enter` | 选择或进入操作确认 |
| `Esc` | 返回上一层或退出 |
| `Ctrl+C` | 退出菜单并恢复终端光标 |

菜单会标出“当前使用”“已启用”“未启用”或“状态未知”，并提供“从当前客户端配置导入卡片”和“查看配置文件路径”。执行写入前会显示完整目标及配置路径；取消确认不会执行该操作。

交互菜单要求输入、输出和错误输出都连接到真实终端，且 `TERM` 不是 `dumb`。在管道、重定向或脚本中，请使用命令模式。`--json` 不会启动菜单。

### 常用命令

```sh
ccsl status                          # 查看全部工具概览
ccsl list codex                      # 查看 Codex 卡片和真实 ID
ccsl paths codex                     # 查看实际配置路径
ccsl use codex PROVIDER_ID            # 应用一张 Codex 卡片
ccsl enable opencode PROVIDER_ID      # 写入一个并存式供应商
ccsl disable opencode PROVIDER_ID     # 移出客户端配置，保留卡片
ccsl import live claude               # 从当前 Claude Code 配置导入
ccsl --help                          # 总帮助
ccsl provider --help                 # 卡片管理帮助
ccsl import file --help              # 文件导入帮助
ccsl --lang en status                # 英文输出
```

`PROVIDER_ID` 请替换为对应工具下的 ID。通常不需要用 `sudo` 运行 `ccsl`；使用其他用户身份会改变默认 HOME 和配置位置。

### 新增和更新供应商卡片

下面以 Claude Code 为例。将内容保存为私有文件 `provider.json`，在本地编辑器中替换示例地址和密钥：

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
ccsl provider add claude --file provider.json     # 只保存卡片
ccsl list claude                                # 确认卡片及 ID
ccsl use claude my-claude                        # 需要使用时再切换
```

也可以在首次添加时使用 `--activate`，一次完成保存和应用；同一 ID 已存在时应使用 `update`，不要重复 `add`。

```sh
ccsl provider update claude --file provider.json # 按文件里的 ID 更新
cat provider.json | ccsl provider update claude  # 也支持标准输入
ccsl provider delete claude PROVIDER_ID          # 删除前先确认生效状态
```

`update` 接收的是完整卡片，不是局部 JSON 补丁；请保留需要的字段。不同工具的 `settingsConfig` 结构不同，例如 Codex 使用 `{ "auth": { ... }, "config": "TOML 原文" }`。可以先在 App 中使用对应工具的预设，或导入已有配置。

卡片输入上限为 1 MiB。请通过文件或标准输入提供密钥，避免把真实密钥放在命令参数、Shell 历史或公开问题反馈中。

### JSON 输出与脚本

```sh
ccsl status --json
ccsl list codex --json
ccsl paths --json
```

成功结果写入标准输出，基本结构为 `{"ok":true,"data":...}`；失败结果写入标准错误，结构为 `{"ok":false,"error":{"code":"...","message":"..."}}`。`data` 随命令变化，脚本应依据退出码和稳定错误码处理结果。

| 退出码 | 含义 |
| --- | --- |
| `0` | 操作成功，或正常显示帮助 |
| `1` | 读取、写入、网络等执行失败 |
| `2` | 参数、输入或操作前置条件不正确，例如工具模式不匹配、试图删除当前卡片 |
| `130` | 交互菜单被 `Ctrl+C` 中断 |

`--lang` 只改变人读输出和帮助，不改变 JSON 字段、英文错误消息或退出码。列表与状态输出不包含供应商配置原文或登录令牌。并存式工具读取失败时，`enabled` / `inLive` 可能为 `null`，应按“未知”处理。

`--json` 后仍需给出具体命令；设备码登录会额外在标准错误输出 `device_login` 事件。普通文本输出支持 `head` 等提前关闭管道的用法；JSON 输出管道提前关闭按执行失败处理。更多细节见 [CLI 完整说明](crates/cli/README.md)。

### 在隔离目录试用

如果只想先熟悉命令，可以为 CLI 指定一个全新的临时 HOME：

```sh
CCSL_DEMO_HOME=$(mktemp -d)
ccsl --home "$CCSL_DEMO_HOME" status
ccsl --home "$CCSL_DEMO_HOME" menu
```

后续每条试用命令都要带上同一个 `--home`。它使用该目录的默认布局，忽略 `CODEX_HOME` 等工具环境变量，但仍会应用该目录内已有的 App 目录覆盖设置，因此完整隔离应从空目录开始。真实账号登录会访问外部服务，试用菜单本身不需要登录。

## 从 cc-switch 迁移

CC Switch Lite 的数据目录为 `~/.cc-switch-lite/`，cc-switch 通常使用 `~/.cc-switch/`。导入时只读取源数据，保存到 CC Switch Lite 的库中；迁移不会自动切换当前供应商。

| 数据来源 | App 入口 | CLI 入口 | 导入内容 |
| --- | --- | --- | --- |
| 某个 AI 工具当前的配置 | 工具页 → 从当前配置导入 | `ccsl import live TOOL` | 从客户端配置生成卡片 |
| 本机 cc-switch 数据目录 | 从 cc-switch 导入 → 本机数据目录 | 首版请使用 App；CLI 没有目录扫描子命令 | 支持的卡片；选中 Codex 时可一并导入兼容的托管账号 |
| cc-switch 导出文件 | 从 cc-switch 导入 → 导出文件 | `ccsl import file FILE` | 支持的供应商卡片，不导入 OAuth 登录凭据 |

### 本机数据目录

1. 打开 App 左侧“从 cc-switch 导入”，选择“本机数据目录”。
2. 目录留空使用默认位置，也可填写自定义绝对路径。
3. 点击“扫描”，检查可导入的工具、卡片数和账号数。
4. 选择工具并确认导入；更换来源目录后需要重新扫描。
5. 到对应工具页面确认卡片，再决定是否切换或启用。

支持读取 `cc-switch.db` 或旧版 `config.json`。已有卡片不会被覆盖；需要本地代理、路由或专用登录的部分条目会被跳过，预览会显示原因。

### 导出文件

支持 `.sql` SQL 导出、`.db` SQLite 快照和旧版 `.json` 配置。CLI 默认只预览，明确加 `--apply` 才写入卡片库：

```sh
ccsl import file backup.sql --preview
ccsl import file backup.sql --apply --tool claude,codex
ccsl import file backup.sql --apply --duplicate rename --tool codex
```

`--tool` 可以重复指定，也可以用逗号分隔；省略时处理支持的全部工具。

- `--duplicate skip`：默认策略，跳过已存在的同 ID 卡片。
- `--duplicate rename`：同 ID 且配置不同时，改名后导入；相同配置仍跳过。

文件导入不会应用新卡片，不导入 OAuth 登录凭据，并会清理推广链接。它用于迁移卡片，不等同于完整账号备份恢复。首版 CLI 提供导入命令，没有导出命令；CC Switch Lite 自身的数据备份见下文。

## Codex 登录与多账号

### 使用本机已有的 ChatGPT 登录

Codex 官方卡可以选择“本机的 ChatGPT 登录（自动）”。应用卡片时使用本机 `auth.json` 或登录暂存中的登录；第三方供应商卡通常使用自己的 API 配置。

“设置 → 切到第三方时保留 Codex 的 ChatGPT 登录”默认关闭：切到第三方时会暂存本机登录并从 `auth.json` 移出，切回采用本机登录的官方卡时恢复。开启后可以保留登录，实际请求行为仍由客户端配置决定。

这里的登录操作针对 `ccsl paths codex` 显示的配置目录；使用前请确认它就是要管理的 Codex 客户端目录。

### 托管多个 ChatGPT 账号

App 中的流程：

1. 打开“Codex 账号”，点击“登录新账号”。
2. 按界面给出的登录网址和短验证码，在浏览器中完成确认。
3. 等待账号出现在列表中。
4. 回到 Codex 工具页，添加或编辑一张官方卡，在“登录方式”中选择指定托管账号或“默认账号”。
5. 保存并按需切换到该卡；编辑当前生效的卡时，保存会更新客户端配置。

CLI 也可以管理账号：

```sh
ccsl accounts list
ccsl accounts login
ccsl accounts login --timeout 600
ccsl accounts default ACCOUNT_ID
ccsl accounts login --account ACCOUNT_ID
ccsl accounts refresh ACCOUNT_ID
ccsl accounts remove ACCOUNT_ID
```

`ACCOUNT_ID` 替换为 `accounts list` 中的 ID。登录默认等待 300 秒，`--timeout` 可设为 1–1800 秒；超时后重新执行会生成新的登录流程。

“设为默认”更新托管账号库，供绑定默认账号的卡片下次应用时使用；它本身不会立即切换客户端。CLI 完成登录后，仍需应用绑定该账号的官方卡。

账号令牌保存在本机，界面和 CLI 账号列表不输出令牌。移除托管账号不会在 OpenAI 服务端注销或撤销该账号，绑定它的卡片需要重新指定可用登录。后台定期刷新由运行中的 App 提供，CLI 没有常驻刷新服务。

## 配置目录与数据备份

### 查看实际生效路径

以 App 工具页中的“配置文件”或 CLI 输出为准：

```sh
ccsl paths
ccsl paths codex
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

普通运行会遵循 `CODEX_HOME`、`HERMES_HOME`、`PI_CODING_AGENT_DIR`、`MINIMAX_DATA_DIR` / `MAVIS_DATA_DIR` 等已支持的路径环境变量，再应用 App 保存的目录覆盖。自定义配置目录时，建议在 App 中设置后运行 `ccsl paths` 核对。

### 本应用保存哪些数据

默认在 `~/.cc-switch-lite/`：

| 文件 / 目录 | 用途 |
| --- | --- |
| `providers.json` | 供应商卡片、当前选择和相关配置设置 |
| `app-settings.json` | App 语言、目录覆盖、托盘及后台刷新等设置 |
| `codex_oauth_auth.json` | 托管 Codex 账号及登录凭据 |
| `codex-login-stash.json` | 切换时暂存的本机 Codex 登录 |
| `codex_managed_oauth_live_auth.json` | 当前托管登录与客户端的对应标记 |
| `live-state.json` | 已完成及尚未完成的写入状态 |
| `backups/live-first-write/` | 每个客户端文件首次写入前的原始备份及来源路径记录 |

数据文件可能包含 API Key 或登录凭据，不是加密保险箱。请将它们作为私有数据保存，分享截图、日志或提交 Issue 前移除敏感内容。

### 手动备份

先从托盘退出 App，并结束正在写配置的 CLI 命令。下面的命令备份默认数据目录到一个带时间戳的用户目录：

```sh
(
  umask 077
  CCSL_BACKUP_DIR="$HOME/ccsl-backup-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$CCSL_BACKUP_DIR"
  cp -a "$HOME/.cc-switch-lite" "$CCSL_BACKUP_DIR/"
)
```

如需恢复“当前客户端正在使用的完整配置”，还应另外备份 `ccsl paths` 显示的客户端文件。首次写入备份只保存每个路径第一次写入前的状态，不是每次切换的历史版本。

备份目录中的 `.source` 文件记录原始路径；原文件当时不存在时，可能只有来源记录而没有原始文件备份。手工恢复前，请先退出相关客户端并保存当前文件，再核对路径和备份内容。首版没有一键还原任意历史版本的命令。

### 中断恢复

App 启动和相关写操作会处理未完成的配置写入。CLI 的 `status`、`list`、`paths` 和文件导入预览保持只读，不会自动执行恢复；需要显式处理时使用：

```sh
ccsl recover codex   # 只处理 Codex
ccsl recover         # 处理所有工具
```

`recover` 用于补完或清理中断的操作，**不表示撤销上一次切换**。如果恢复提示某些文件已被外部修改或只能部分恢复，请通过 `paths` 定位后检查；不要直接覆盖仍在使用的新配置或新登录凭据。

## 常见问题

### `ccsl: command not found`

确认已经将二进制放入 PATH，并重新打开终端。也可以先用绝对路径或 `./ccsl-0.1.0/ccsl --version` 验证文件能否运行，参见 [CLI 安装步骤](#3-安装独立-cli)。

### `Permission denied`、`Exec format error` 或 `Bad CPU type`

确认下载的操作系统和 CPU 架构正确。压缩包保留执行权限；如果复制过程中丢失，可对自己的 `ccsl` 文件执行 `chmod +x ccsl`。Linux 包使用 glibc，不能假定可直接用于 Alpine 等 musl 环境。

### macOS 提示无法验证开发者或文件已损坏

本版 macOS App 和 CLI 均未做 Apple Developer ID 签名与公证。Homebrew 安装完成不代表已获得系统运行许可，首次打开 App 或执行 `ccsl` 都可能被拦截。

先确认下载来源与 SHA256，再到“系统设置 → 隐私与安全性”找到刚刚被拦截的 **CC Switch Lite** 或 **ccsl**，按提示选择“仍要打开”，然后重新启动 App 或执行 CLI。详见 [Apple 官方说明](https://support.apple.com/zh-cn/102445)。安装器不会替用户关闭系统安全检查。

若提示文件已损坏或校验不一致，先重新下载并核对 SHA256；仍有问题时附上系统版本及完整提示反馈。

### 工具显示“未检测到配置文件”

这表示预期位置的配置文件尚不存在，不代表已检测到或安装了那个 AI 工具。先运行 `ccsl paths TOOL` 核对位置；自定义目录应在设置中填写。必要时先在对应客户端完成首次启动或登录。

### 导入成功了，为什么配置没切换？

从 cc-switch 迁移只保存卡片。进入工具页点击“切换”或启用开关，或执行 `ccsl use TOOL PROVIDER_ID` / `ccsl enable TOOL PROVIDER_ID`。新增卡片默认也只保存，除非明确选择立即应用。

### “从当前配置导入”显示没有新条目

切换式工具只在卡片库为空时进行首次导入；已有卡片时会保持原样。并存式工具只补齐缺少的条目。要修改已有卡片，请使用编辑器或 `provider update`。

### `invalid_tool_mode`，或菜单无法打开

切换式工具使用 `use`，并存式工具使用 `enable` / `disable`。菜单需要交互终端；管道、重定向、`TERM=dumb` 或 `--json` 场景请改用具体命令。

### 提示配置读取失败、状态未知、写入冲突或权限错误

先运行 `ccsl paths TOOL` 找到实际文件，检查其 JSON / TOML / YAML 格式、写入权限，以及是否有其他程序同时修改。修复后重试读取。App 和 CLI 的跨进程锁用于协调本应用的操作，不能阻止其他编辑器或客户端改写文件。

### 显示已切换，但客户端仍使用原供应商

重新加载客户端配置或开启新会话，并检查是否使用了其他配置目录、进程环境变量或启动参数。“当前选择”是本应用保存的状态；本应用不会向供应商发送测试请求来验证密钥、余额或模型可用性。

### Codex 提示需要重新登录，或出现 `codex.oauth.live_sync_failed`

“需要重新登录”时，在 App 账号页重新登录，或执行 `ccsl accounts login --account ACCOUNT_ID`。

`codex.oauth.live_sync_failed` 表示新凭据已保存到账号库，但同步客户端失败。检查 Codex 配置路径和权限后，再应用对应官方卡；不要用旧 token 覆盖已保存的新凭据。

### 关闭 App 窗口后为什么还在运行？

默认开启“关闭窗口时留在托盘”。可从托盘选择“退出”，或在设置中关闭此选项。CLI 本身无需桌面 App 常驻。

### 如何反馈问题？

在 [Issues](https://github.com/Ricardo121380/ccswitch-lite/issues) 中提供：App / CLI 版本、操作系统与架构、使用的工具、复现步骤、预期结果和实际错误码。路径可脱敏；请勿附上真实 API Key、OAuth token 或完整账号库。

## 升级与卸载

通过 Homebrew 安装的版本使用上面的 `brew upgrade` 命令升级。npm 方式先更新安装器包，再按需执行 `ccsl-app install --replace`；仅更新 npm 包不会自动替换桌面 App。

手动下载的版本，升级时先退出 App 和 CLI 交互菜单，再下载对应平台的新版本：macOS 替换“应用程序”中的 App；Linux 使用 `apt install ./新版本.deb`；CLI 使用新二进制替换原安装位置。升级前建议备份本应用数据和客户端配置。

卸载 App 或 CLI 二进制不会自动还原已经写入客户端的供应商配置。若希望回到官方或其他供应商，请在卸载前先完成切换。

- Homebrew：使用 `brew uninstall --cask ricardo121380/ccswitch-lite/ccsl` 或 `brew uninstall --cask ricardo121380/ccswitch-lite/ccswitch-lite`。
- npm：使用 `npm uninstall -g ccswitch-lite` 移除两个命令入口；它不会移除已经安装的桌面 App。
- 手动安装的 macOS App：从对应的“应用程序”目录移除 CC Switch Lite。
- Linux DEB：使用 `sudo apt remove cc-switch-lite`。
- CLI：移除自己安装的 `ccsl` 二进制。

`~/.cc-switch-lite/` 中的卡片、账号和备份由用户自行保留或清理。删除前请确认不再需要其中的登录暂存和备份。

## 从源码构建与开发

只使用发布包的用户可以跳过这一节。

### 构建独立 CLI

需要 Rust stable 和平台 C/C++ 构建工具。工作区声明的 `rust-version` 为 `1.89`，CI 使用 stable 工具链。

```sh
git clone https://github.com/Ricardo121380/ccswitch-lite.git
cd ccswitch-lite
cargo build -p ccsl-cli --release --locked
./target/release/ccsl --version
./target/release/ccsl
```

该命令只构建 CLI，不需要安装 Tauri 桌面依赖或前端依赖。

### 开发和打包桌面 App

在 Rust 和平台编译工具之外，需要 Node.js、pnpm 以及 Tauri 的系统依赖。本项目 CI 使用 **Node.js 24** 和 **pnpm 11.22.0**；macOS 需要 Xcode Command Line Tools。

Ubuntu 24.04 的桌面构建依赖与 CI 一致：

```sh
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential libssl-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

在仓库根目录执行：

```sh
pnpm install --frozen-lockfile --ignore-scripts
pnpm typecheck
pnpm test
pnpm tauri dev
```

**直接运行 `pnpm tauri dev` 会使用当前用户的真实配置。** 需要演示或试验时，请使用下面的隔离环境。

正式打包命令：

```sh
# macOS；CI=true 适用于不依赖 Finder 窗口排版的打包环境
CI=true pnpm tauri build --ci --bundles app,dmg -- --locked

# Linux
pnpm tauri build --ci --bundles deb -- --locked
```

产物位于 `target/release/bundle/`。macOS 配置见 [tauri.macos.conf.json](crates/app/tauri.macos.conf.json)；Linux 默认配置还包含 AppImage 目标，但 v0.1.0 Release 只分发已验证的 DEB。

### 使用隔离演示数据启动 App

以下示例先构建 App，再用项目提供的假配置填充临时目录；启动时移除会覆盖工具路径的环境变量。

```sh
pnpm tauri build --ci --no-bundle -- --locked
CCSL_DEMO_HOME=$(mktemp -d)
cargo run -p ccsl-app --example seed_demo --locked -- "$CCSL_DEMO_HOME"
env -u CODEX_HOME -u HERMES_HOME -u PI_CODING_AGENT_DIR \
  -u MINIMAX_DATA_DIR -u MAVIS_DATA_DIR \
  HOME="$CCSL_DEMO_HOME" CCSL_HOME="$CCSL_DEMO_HOME" \
  XDG_CONFIG_HOME="$CCSL_DEMO_HOME/.config" \
  target/release/ccswitch-lite
```

演示目录包含假卡片和假账号，不应把它们当作可用的在线服务凭据。

### 检查与发布流程

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p ccsl-cli --locked
python3 scripts/check-cli-menu.py --binary target/release/ccsl
```

测试及交互菜单检查使用临时 HOME。菜单脚本面向 macOS / Linux，需要 Python 3，并需先构建 CLI。

- [检查工作流](.github/workflows/ci.yml)：主分支 push 和 PR 时运行类型检查、前端测试、Rust 测试、Clippy，以及 Linux / macOS 桌面打包验证；CLI 单独构建、检查并上传压缩包。
- [发布工作流](.github/workflows/release.yml)：版本标签触发正式 App / CLI 构建，校验版本与架构，生成 SHA256 文件并创建草稿 Release，再确认发布。包含分发工作流的后续版本还会生成 npm 安装器附件；不自动发布 npm 官方仓库。
- [v0.1.0 发布说明](docs/releases/v0.1.0.md)：首版功能与平台范围。
- [分发维护说明](docs/distribution.md)：更新 Homebrew Cask、生成 npm 包和手动发布流程。

重新生成内置预设时，使用本机的上游源码目录：

```sh
pnpm presets /absolute/path/to/cc-switch
```

预设生成脚本仅保留支持的直连配置和模型厂商官方链接。域名白名单见 [official-vendor-domains.json](ui/src/lib/official-vendor-domains.json)，移除记录见 [removed-preset-links.json](ui/src/lib/removed-preset-links.json)。

## 项目结构与许可证

| 路径 | 内容 |
| --- | --- |
| [`crates/core`](crates/core) | 配置投影与写入、供应商库、首次写入备份、中断恢复、导入和 Codex OAuth；无 Tauri 依赖 |
| [`crates/service`](crates/service) | App / CLI 共用业务层、设置、路径覆盖、脱敏及 OAuth HTTP 实现 |
| [`crates/app`](crates/app) | Tauri 2 桌面外壳、托盘、文件监听及后台账号刷新 |
| [`crates/cli`](crates/cli) | 独立命令行 `ccsl` 及交互菜单 |
| [`ui`](ui) | React、TypeScript、Vite、TanStack Query 前端及中英文界面 |
| [`scripts`](scripts) | 预设生成、CLI 伪终端检查及分发元数据生成脚本 |
| [`Casks`](Casks) | Homebrew 原生 CLI 与 macOS 桌面 App 的二进制 Cask |
| [`packages/npm`](packages/npm) | 零运行时依赖的 npm CLI 启动器与桌面安装器 |

本项目基于 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 的写入引擎重写，保留其 MIT 版权声明。许可证见 [LICENSE](LICENSE)，移植来源与改动说明见 [NOTICE.md](NOTICE.md)。原作版权 © 2025 Jason Young。
