# SwitchLeaf 安装器

SwitchLeaf 是 CC Switch Lite 的新名称。当前锁定的 v0.1.0 安装器仍使用下文的旧包名和旧命令；后续生成的新版本使用 `switchleaf` npm 包，提供 `switchleaf` / `switchleaf-app`，并保留 `ccsl` / `ccsl-app` 别名。新功能以对应原生发布包为准，npm 不额外实现一套功能。


一个 npm 包提供两个命令：

- `ccsl`：原生 CLI，支持方向键菜单和完整命令模式。
- `ccsl-app`：桌面 App 的下载、安装和启动工具。

支持 macOS Apple Silicon 和 Linux x86_64，要求 Node.js 20 或更新版本。原生程序来自本项目 GitHub Release，每个下载都按包内固定的 SHA256 校验。npm 安装阶段不执行下载脚本，使用 `--ignore-scripts` 也能正常安装。

## 安装

npm 官方仓库尚未发布本包。从项目根目录生成本地安装包后使用：

```sh
mkdir -p target/npm
CCSL_NPM_TARBALL=$(npm pack ./packages/npm --ignore-scripts --pack-destination target/npm --silent)
npm install -g "./target/npm/$CCSL_NPM_TARBALL" --ignore-scripts
ccsl --version
ccsl
```

已有 `.tgz` 文件时，也可直接执行 `npm install -g /absolute/path/to/ccswitch-lite-VERSION.tgz --ignore-scripts`。VERSION 替换为实际版本。

首次运行 `ccsl` 会下载该版本对应的原生二进制，随后从用户缓存启动；参数、标准输入、JSON 输出和终端按键保持原 CLI 行为。

## 安装桌面 App

```sh
ccsl-app install
ccsl-app open
```

macOS 默认安装到 `~/Applications/CC Switch Lite.app`。需要其他位置时使用绝对路径：

```sh
ccsl-app install --app-dir /Applications
ccsl-app open --app-dir /Applications
```

Linux 使用 `apt install` 安装 DEB，普通用户通过 `sudo` 调用；需要终端确认和系统密码，图形桌面启动 App 时仍使用普通用户。请勿用 sudo 运行 `ccsl` 或桌面 App。该 Linux 包在 Ubuntu 24.04 构建和验证，不适用于所有发行版。

macOS App 和 CLI 均未做 Apple Developer ID 签名和公证。首次运行时可能需要在系统“隐私与安全性”中手动允许，详见[项目安装说明](https://github.com/Ricardo121380/ccswitch-lite#下载与安装)。安装完成后不会自动启动 App。

## 下载、路径与升级

```sh
ccsl-app download --output ./downloads
ccsl-app path
ccsl-app --help
```

更新 npm 安装包后，CLI 会自动使用包内固定的新版本。桌面 App 需要先退出，再执行：

```sh
ccsl-app install --replace
```

macOS 仅允许替换具有相同 bundle identifier 的 App；非本项目应用和符号链接会被拒绝。配置数据位于 `~/.cc-switch-lite/`，与安装文件、下载缓存分开。

缓存默认位于 `~/.cache/ccswitch-lite/`，也支持绝对路径的 `XDG_CACHE_HOME`。删除缓存后，下次使用会重新下载。卸载 npm 包只会移除命令入口，已安装的桌面 App 与用户数据保留。

如果也通过 Homebrew 安装了 CLI，请用 `command -v ccsl` 确认当前实际运行的入口。

完整用法：[用户指南](https://github.com/Ricardo121380/ccswitch-lite#readme) · [原生 CLI 文档](https://github.com/Ricardo121380/ccswitch-lite/blob/main/crates/cli/README.md)
