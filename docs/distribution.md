# Homebrew 与 npm 分发维护

Homebrew Cask 与 npm 安装器共用 GitHub Release 的原生二进制。它们不构建另一套 App 或 CLI，也不会修改用户的 `~/.leanflip/` 数据。

## 名称

发布附件使用 `leanflip_VERSION_PLATFORM` 文件名：CLI 压缩包里是 `leanflip`，桌面包里是 `LeanFlip.app`（主二进制 `leanflip-app`）。`prepare-distribution.mjs` 生成 `leanflip` npm 包（`leanflip` / `leanflip-app` 两个命令）和同名的两个 Homebrew Cask。桌面二进制名称和应用 identifier 直接取自 `crates/app/tauri.conf.json`，安装器和 Cask 不会与实际构建不一致。npm 安装器所需的全部名称都写在 `release.json` 里，代码里没有写死。

`packages/npm/release.json` 和 `Casks/` 必须与某个已发布的原生版本一一对应：发布后由脚本从正式附件生成并提交，不要手改。仓库里没有 `release.json` 时，`Package managers` 工作流只运行安装器单元测试，跳过真实下载和 Homebrew 安装。生成时脚本会删除不属于本次发布的 Cask，`--check` 发现残留会报错；v0.1.0 不能重新生成。

发布前需确定新版本号、生成对应发布说明和原生资产，再按下面步骤校验与发布。

## 当前入口

- Homebrew tap：本仓库本身，`brew tap ricardo121380/leanflip https://github.com/Ricardo121380/LeanFlip.git`。
- CLI Cask：`Casks/leanflip.rb`（发布新版本时生成），macOS ARM64 / Linux x64；Linux 需要 Homebrew 7+。
- 桌面 Cask：`Casks/leanflip-app.rb`（发布新版本时生成），macOS ARM64。
- npm 包源：`packages/npm`，包名 `leanflip`，命令为 `leanflip` 和 `leanflip-app`。
- npm 官方仓库：[`leanflip`](https://www.npmjs.com/package/leanflip)，从 0.2.1 起发布；0.2.1 由维护者手动发布；从 0.2.2 起由 `npm-publish.yml` 在正式发布 Release 时自动发布（带来源证明）。

两个 Homebrew 入口都用 `brew install --cask` 安装，避免预编译 CLI 被作为源码配方触发编译工具检查。

macOS 的 App 和 CLI 当前均未做 Developer ID 签名及公证。真实用户环境中的 Gatekeeper 可能拦截首次运行；CI 安装和执行通过不能证明用户电脑已授予运行许可。保留 Homebrew 的隔离属性和系统安全检查，按 README 引导用户手动处理首次启动提示。

桌面主二进制和 npm 桌面命令都叫 `leanflip-app`，与 CLI 的 `leanflip` 分开；Linux DEB 安装到 `/usr/bin/leanflip-app`。

## 更新原生版本与校验值

先完成新的原生 Release 构建，把四份原生附件及 `SHA256SUMS` 下载到一个目录。例如：

```sh
mkdir -p target/distribution-assets
gh release download vVERSION --dir target/distribution-assets
node scripts/prepare-distribution.mjs VERSION target/distribution-assets
```

版本号、目录按实际发布调整。生成脚本会核对附件本身的 SHA256，然后生成对应版本的 Homebrew Cask、npm 包版本和 `release.json`，并复制当前 LICENSE 到 npm 包。不要手工猜测或关闭 SHA256 校验。

```sh
node scripts/prepare-distribution.mjs VERSION target/distribution-assets --check
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

首次运行 `leanflip` 时下载并校验原生 CLI，缓存后转交完整参数、标准输入输出和终端交互。桌面安装器通过 `leanflip-app install` 显式运行；macOS 复制到用户应用目录，Linux 调用系统 apt 安装。缓存默认位于 `~/.cache/leanflip/`，可通过绝对路径 `XDG_CACHE_HOME` 调整。

`.github/workflows/distribution.yml` 会在 macOS 和 Linux 打包并安装 npm 包，运行原生 CLI 和伪终端菜单检查；同时验证桌面下载、macOS 安装与替换，以及 Homebrew 实际安装。

发布工作流会在 Release 草稿中附带与同版本原生程序匹配的 npm `.tgz`，并纳入 `SHA256SUMS`。它不调用 `npm publish`；发布到 npm 由正式发布 Release 后的 `npm-publish.yml` 完成，见下一节。

## 发布到 npm 官方仓库

正式发布 GitHub Release（把草稿点成 Published）时，`.github/workflows/npm-publish.yml` 会下载该 Release 附带的 `leanflip-VERSION.tgz`，核对 `SHA256SUMS`、包名和版本号，再发布到 npm。npm 通过“可信发布”（Trusted Publishing）信任这个工作流：仓库和工作流里不保存任何 npm 令牌，发布时自动生成来源证明（provenance）。已经在 npm 上的版本会跳过；预发布版本不会发布。

npm 端的配置在 [leanflip 的 Access 设置页](https://www.npmjs.com/package/leanflip/access) 的 Trusted Publisher 里，选 GitHub Actions，字段区分大小写：

| 字段 | 值 |
| --- | --- |
| Organization or user | `Ricardo121380` |
| Repository | `LeanFlip` |
| Workflow filename | `npm-publish.yml` |
| Environment name | 留空 |
| Allowed actions | 勾选 **Direct npm publish** |

新建的配置默认只允许暂存发布（staged publish），不勾选 Direct npm publish 时工作流会得到 `403 OIDC permission denied for this action`。新建的配置必须在 2 天内完成第一次成功发布，否则会失效，需要删除后重建。因此在准备正式发布某个版本时再添加，然后发布草稿。第一次自动发布成功后，可以在同一页把 Publishing access 改为 “Require two-factor authentication and disallow tokens”，只保留可信发布这条路径。

工作流要求 npm 11.5.1 及以上、Node 22.14 及以上（工作流用 Node 24 并固定 npm 11.12.1）。`packages/npm/package.json` 的 `repository.url` 必须指向本仓库。Release 必须由维护者账号发布：用 `GITHUB_TOKEN` 发布的 Release 不会触发其他工作流。

自动发布失败、或需要补发时，可以在 Actions 页手动运行 “Publish to npm” 并填写标签，例如 `v0.2.1`。也可以由维护者在本机发布同一个 `.tgz`，npm 会在浏览器中要求二次验证。不要将密码、令牌或验证码写入仓库、工作流或聊天记录：

```sh
npm login --registry=https://registry.npmjs.org/
npm publish ./leanflip-VERSION.tgz --access public --registry=https://registry.npmjs.org/
```

不要在仓库根目录直接执行 `npm publish`（根 `package.json` 是私有的前端开发项目）。发布后用 `npm view leanflip version` 检查。

## 卸载与替换边界

- Homebrew 正常卸载只移除所安装的程序，不配置 `zap` 删除用户数据。
- `npm uninstall -g leanflip` 只移除命令入口，桌面 App、缓存和用户配置保留。
- macOS 安装器拒绝替换无关 App、符号链接或未明确指定 `--replace` 的已有安装；替换前需要退出 App。
- App 替换先复制到临时目录并检查 bundle identifier，再替换原位置；失败时尝试恢复旧 App，恢复失败则保留备份。
