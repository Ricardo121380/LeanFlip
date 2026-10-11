# Homebrew 与 npm 分发维护

Homebrew 配方与 npm 安装器共用 GitHub Release 的原生二进制。它们不构建另一套 App 或 CLI，也不会修改用户的 `~/.cc-switch-lite/` 数据。

## 当前入口

- Homebrew tap：本仓库本身，`brew tap ricardo121380/ccswitch-lite https://github.com/Ricardo121380/ccswitch-lite.git`。
- CLI Formula：`Formula/ccsl.rb`，macOS ARM64 / Linux x64。
- 桌面 Cask：`Casks/ccswitch-lite.rb`，macOS ARM64。
- npm 包源：`packages/npm`，包名 `ccswitch-lite`，命令为 `ccsl` 和 `ccsl-app`。
- npm 官方仓库尚未发布；不要在用户说明中将 `npm install -g ccswitch-lite` 写成当前可用命令。

桌面 npm 命令使用 `ccsl-app`，避免与 Linux DEB 安装的原生 `/usr/bin/ccswitch-lite` 重名。

## 更新原生版本与校验值

先完成新的原生 Release 构建，把四份原生附件及 `SHA256SUMS` 下载到一个目录。例如：

```sh
mkdir -p target/distribution-assets
gh release download v0.1.0 --dir target/distribution-assets
node scripts/prepare-distribution.mjs 0.1.0 target/distribution-assets
```

版本号、目录按实际发布调整。生成脚本会核对附件本身的 SHA256，然后更新 Homebrew 配方、Cask、npm 包版本和 `release.json`，并复制 LICENSE / NOTICE 到 npm 包。不要手工猜测或关闭 SHA256 校验。

```sh
node scripts/prepare-distribution.mjs 0.1.0 target/distribution-assets --check
node --test packages/npm/test/*.test.mjs
```

`--check` 只检查已生成文件是否与附件一致，不写入文件。检查 diff 后，将生成文件提交并推送到 main；用户后续的 `brew update` / `brew upgrade` 才会看到新版本。

## 生成 npm 包

在仓库根目录运行：

```sh
mkdir -p target/npm
npm pack ./packages/npm --ignore-scripts --pack-destination target/npm
```

包内只包含两个启动入口、安装器、版本与校验元数据、文档和许可证。没有运行时 npm 依赖，没有 `postinstall`，不需要执行安装脚本。

首次运行 `ccsl` 时下载并校验原生 CLI，缓存后转交完整参数、标准输入输出和终端交互。桌面安装器通过 `ccsl-app install` 显式运行；macOS 复制到用户应用目录，Linux 调用系统 apt 安装。缓存默认位于 `~/.cache/ccswitch-lite/`，可通过绝对路径 `XDG_CACHE_HOME` 调整。

`.github/workflows/distribution.yml` 会在 macOS 和 Linux 打包并安装 npm 包，运行原生 CLI 和伪终端菜单检查；同时验证桌面下载、macOS 安装与替换，以及 Homebrew 实际安装。

包含新发布工作流的后续版本，会在 Release 草稿中附带与同版本原生程序匹配的 npm `.tgz`，并纳入 `SHA256SUMS`。该工作流不调用 `npm publish`。v0.1.0 的既有原生附件无需替换；这次仅准备本地 npm 包。

## 首次发布到 npm 官方仓库

此步骤需要维护者另行决定发布，并在自己的终端完成 npm 登录。不要将密码、令牌或验证码写入仓库、工作流或聊天记录。

```sh
npm login --registry=https://registry.npmjs.org/
npm whoami --registry=https://registry.npmjs.org/
npm publish ./target/npm/ccswitch-lite-0.1.0.tgz --access public --registry=https://registry.npmjs.org/
```

发布时使用经过验证的 `.tgz`，不要在仓库根目录直接执行 `npm publish`（根 `package.json` 是私有的前端开发项目）。npm 可能要求维护者在浏览器中完成二次验证。

成功后，用 `npm view ccswitch-lite version` 检查，再更新 README 中的发布状态，才能将 `npm install -g ccswitch-lite` 作为用户入口。后续如需 GitHub Actions 自动发布，可在 npm 包设置中配置对应仓库与工作流的 Trusted Publisher；当前未配置这一权限，也未保存发布令牌。

## 卸载与替换边界

- Homebrew 正常卸载只移除所安装的程序，不配置 `zap` 删除用户数据。
- `npm uninstall -g ccswitch-lite` 只移除命令入口，桌面 App、缓存和用户配置保留。
- macOS 安装器拒绝替换无关 App、符号链接或未明确指定 `--replace` 的已有安装；替换前需要退出 App。
- App 替换先复制到临时目录并检查 bundle identifier，再替换原位置；失败时尝试恢复旧 App，恢复失败则保留备份。
