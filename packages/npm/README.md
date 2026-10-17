# LeanFlip 安装器

npm 包 `leanflip` 下载并校验 LeanFlip 的原生程序，功能以对应的原生发布包为准。

一个 npm 包提供两个命令：

- `leanflip`：原生 CLI，支持方向键菜单和完整命令模式。
- `leanflip-app`：桌面 App 的下载、安装和启动工具。

支持 Windows 10 / 11 x64、macOS Apple Silicon 和 Linux x86_64，要求 Node.js 20 或更新版本。原生程序来自本项目 GitHub Release，每个下载都按包内固定的 SHA256 校验。npm 安装阶段不执行下载脚本，使用 `--ignore-scripts` 也能正常安装。

## 安装

```sh
npm install -g leanflip
leanflip --version
leanflip
```

首次运行 `leanflip` 会下载该版本对应的原生二进制，随后从用户缓存启动；参数、标准输入、JSON 输出和终端按键保持原 CLI 行为。

## 安装桌面 App

```sh
leanflip-app install
leanflip-app open
```

macOS 默认安装到 `~/Applications/LeanFlip.app`。需要其他位置时使用绝对路径：

```sh
leanflip-app install --app-dir /Applications
leanflip-app open --app-dir /Applications
```

Windows 会打开 NSIS 安装向导，默认安装在 `%LOCALAPPDATA%\LeanFlip`；`path` / `open` 使用该默认位置，自定义位置的 App 请从开始菜单启动。安装程序未代码签名，可能显示 SmartScreen 提示。CLI 解压为原生 `leanflip.exe`，不依赖 WSL。

Linux 使用 `apt install` 安装 DEB，普通用户通过 `sudo` 调用；需要终端确认和系统密码，图形桌面启动 App 时仍使用普通用户。请勿用 sudo 运行 `leanflip` 或桌面 App。该 Linux 包在 Ubuntu 24.04 构建和验证，不适用于所有发行版。

macOS App 和 CLI 均未做 Apple Developer ID 签名和公证。首次运行时可能需要在系统“隐私与安全性”中手动允许，详见[项目安装说明](https://github.com/Ricardo121380/LeanFlip#安装)。Windows 安装向导可选择完成后启动 App；其他平台安装器不会自动启动。

## 下载、路径与升级

```sh
leanflip-app download --output ./downloads
leanflip-app path
leanflip-app --help
```

更新 npm 安装包后，CLI 会自动使用包内固定的新版本。桌面 App 需要先退出，再执行：

```sh
leanflip-app install --replace
```

macOS 仅允许替换具有相同 bundle identifier 的 App；非本项目应用和符号链接会被拒绝。配置数据位于 `~/.leanflip/`（旧版本的数据目录会在首次写入时自动迁移），与安装文件、下载缓存分开。

缓存默认位于 `~/.cache/leanflip/`，也支持绝对路径的 `XDG_CACHE_HOME`。删除缓存后，下次使用会重新下载。卸载 npm 包只会移除命令入口，已安装的桌面 App 与用户数据保留。

如果也通过 Homebrew 安装了 CLI，请用 `command -v leanflip` 确认当前实际运行的入口。

完整用法：[用户指南](https://github.com/Ricardo121380/LeanFlip#readme) · [原生 CLI 文档](https://github.com/Ricardo121380/LeanFlip/blob/main/crates/cli/README.md)
