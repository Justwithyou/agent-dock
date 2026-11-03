# AgentDock

[![CI](https://github.com/Justwithyou/agent-dock/actions/workflows/ci.yml/badge.svg)](https://github.com/Justwithyou/agent-dock/actions/workflows/ci.yml)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](Cargo.toml#L12)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](Cargo.toml#L14)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

AgentDock 是面向研发团队的 AI 能力资产管理工具：以 Git 为唯一事实源，统一管理 Prompt 与 Skill，并通过同一 Rust Core 为 `adock` CLI 和 Tauri 桌面端提供版本管理与安全分发能力。

## 当前能力

- Prompt / Skill 工作区初始化、发现、列表、详情与校验。
- Prompt 变量声明、默认值、必填检查和显式渲染。
- 多文件 Skill 资源读取、依赖声明检查与路径边界校验。
- 基于 Git 的历史、双版本 diff、资产标签、`prod` 通道和单资产回滚。
- CLI 与桌面端共享同一核心，不维护第二套资产或版本算法。
- 分发计划、SHA-256 校验、冲突检测、原子写入、失败重试和目标侧清单。
- Claude Code、Cursor、Trae、Codex、WorkBuddy 首批格式适配器。

> 当前已完成开发阶段 0—10，并通过 Windows MVP 发布验收。CLI 与桌面端均提供完整版本管理与分发闭环，团队协作文档、系统加固、独立发布物和干净环境验收均已补齐。

## 适配器矩阵

| 目标工具 | Prompt | Skill | TargetProfile 格式根示例 |
|---|---:|---:|---|
| Claude Code | `skills/<id>/SKILL.md` | `skills/<id>/SKILL.md` 及资源 | `~/.claude` |
| Cursor | `skills/<id>/SKILL.md` | 未注册 | `~/.cursor` |
| Trae | `commands/<id>.md` | `skills/<id>/SKILL.md` 及资源 | `~/.trae-cn`、`~/.trae` 或项目 `.trae` |
| Codex | 不注册（Custom Prompts 已弃用） | `skills/<id>/SKILL.md` 及资源 | `~/.agents` 或项目 `.agents` |
| WorkBuddy | `commands/<id>.md` | `skills/<id>/SKILL.md` 及资源 | `~/.codebuddy` 或项目 `.codebuddy` |

适配器按 `(asset kind, target)` 独立注册。Prompt 与 Skill 的目录语义不会互相隐式转换。

## 工程结构

```text
crates/
  core/       资产、版本、校验和分发领域模型
  infra/      工作区、配置、Git 快照输入所需服务及分发执行器
  adapters/   Git 命令适配层与各目标工具格式适配器
  cli/        adock 命令行入口
  desktop/    Tauri 桌面端与共享 Core 调用契约
tests/
  fixtures/   测试资产约定
  golden/     适配器黄金文件
```

代码仓库与团队资产仓库分离。应用不需要独立数据库、本地业务 HTTP 服务或常驻进程。

## 环境要求

- Rust 1.85 或更新版本（含 `rustfmt`、`clippy`）。
- Git，且可从命令行访问。
- 桌面开发需要对应平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)；当前验收基线为 Windows。

## 快速开始

克隆代码后，在仓库根目录执行：

```bash
cargo build -p adock-cli
cargo run -p adock-cli -- --help
```

直接使用与工具仓库同级的现有资产工作区并完成校验：

```bash
cargo run -p adock-cli -- --workspace ../agent-dock-assets list
cargo run -p adock-cli -- --workspace ../agent-dock-assets validate
```

只有需要创建另一套全新的资产仓库时，才在空目录中执行 `adock --workspace <PATH> init`。

启动桌面开发版本：

```bash
cargo run -p adock-desktop
```

## Windows 发布物与安装

在仓库根目录构建独立 CLI，再进入桌面 crate 构建 NSIS 安装包：

```powershell
cargo build --release -p adock-cli
Set-Location crates/desktop
npx --yes @tauri-apps/cli@2 build
Pop-Location
```

构建产物：

- CLI：`target/release/adock.exe`（在仓库根目录构建时）。
- 桌面安装包：`target/release/bundle/nsis/AgentDock_0.1.0_x64-setup.exe`。

安装时运行 NSIS 安装包并选择目录，或将 `adock.exe` 复制到已加入 `PATH` 的目录。桌面端需要 Windows WebView2 Runtime；工作区、版本管理和分发操作需要命令行可访问的 Git。Node.js 仅用于执行上述桌面打包命令，不是 CLI 或桌面端运行依赖。

升级前先退出桌面端，并备份资产仓库中的未提交修改与 `adock.local.yaml`。随后用新版安装包覆盖安装、替换 CLI 可执行文件，再检查 `adock --version`、打开原工作区并执行 `adock validate`。升级不会代替 Git 提交、评审或发布流程。

卸载前同样备份需要保留的本机配置。桌面端可通过 Windows“已安装的应用”或安装目录中的 `uninstall.exe` 卸载；CLI 删除复制出的 `adock.exe` 即可。卸载不会自动删除团队资产仓库、目标侧已分发文件或 `.adock/distribution-manifest.json`，需要清理时应先确认这些文件不再使用。

## 常用 CLI 命令

以下命令均可通过全局 `--workspace <PATH>` 指定资产工作区：

```bash
# 查看资产，支持类型、负责人、分类标签和关键词过滤
adock list --kind prompt --owner team --query review

# 查看工作区资产，或读取指定 Git 快照
adock show prompt code-review
adock show prompt code-review --ref adock/prompt/code-review/v1.0.0

# 仅显式传入变量时渲染 Prompt
adock show prompt code-review --var change=PATCH

# 校验整个工作区或单个资产
adock validate
adock validate skill repo-onboarding

# 历史与双版本差异
adock log prompt code-review --limit 10
adock diff prompt code-review --from adock/prompt/code-review/v1.0.0 --to HEAD

# 创建不可变版本标签；标签不会自动推送
adock tag create prompt code-review v1.0.0
adock tag list prompt code-review

# 移动 prod 必须同时显式指定 --move 与 --yes
adock tag create prompt code-review prod --move --yes

# 默认仅预览回滚；--yes 才恢复工作区文件，不自动提交
ROLLBACK_COMMIT="$(git rev-parse "adock/prompt/code-review/v1.0.0^{}")"
adock rollback prompt code-review --to "$ROLLBACK_COMMIT"
adock rollback prompt code-review --to "$ROLLBACK_COMMIT" --yes

# 分发默认使用 prod；dry-run 与执行共用同一计划规则
adock distribute prompt code-review --target trae --var change=PATCH --dry-run
adock distribute prompt code-review --target trae --var change=PATCH --yes
adock distribute skill repo-onboarding --target codex --dry-run

# 仅已纳管本地修改允许显式覆盖；未纳管冲突始终阻断
adock distribute prompt code-review --target trae --var change=PATCH --overwrite-managed --dry-run
```

Revision 支持 `working-tree`、`HEAD`、完整 commit SHA 和 `adock/<kind>/<id>/<name>`。`show` 默认使用工作区，`log` 与标签创建默认使用 `HEAD`，`diff` 和 `rollback` 必须显式提供 revision。

## 资产工作区

```text
agent-dock-assets/
  adock.yaml
  adock.local.yaml       # 可选，本机覆盖，不应提交
  prompts/<id>.md
  skills/<id>/SKILL.md
  skills/<id>/...        # 可选资源，保持原相对结构
  schemas/
```

目标分发路径在共享 `adock.yaml` 或本机 `adock.local.yaml` 中配置。相对路径以资产工作区根目录为基准，`~` 展开为用户主目录；本机同名 TargetProfile 整项覆盖共享配置。

```yaml
default_channel: prod
targets:
  claude-code:
    root: ~/.claude
  trae:
    root: ~/.trae-cn
  codex:
    root: ~/.agents
```

## 分发安全规则

- 正式分发只接受带完整 commit SHA 的快照，不允许未提交的 `working-tree`。
- dry-run 与执行共用同一份计划；执行前重新检查目标与清单是否变化。
- 目标路径必须是可移植相对路径，不能逃逸 TargetProfile 根目录或写入保留的 `.adock/`。
- 未纳管同名文件始终阻断；已纳管文件的本地修改只有显式覆盖才允许更新。
- 单文件原子写入，部分失败明确标记；重新生成计划后可安全重试。
- 清单位于目标根目录 `.adock/distribution-manifest.json`，记录 AssetRef、commit SHA、适配器版本、路径和 SHA-256。
- Skill 脚本与其他资源只复制、不执行；依赖只校验、不递归安装。

## 开发与质量检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

现有 Windows CI 执行相同检查，并额外验证初始化资产。测试使用隔离临时 Git 仓库、Prompt / Skill fixtures 和适配器黄金文件，不依赖开发者本机真实资产。

## 版本与协议

当前版本为 `0.1.0`，处于 MVP 研发阶段，尚未发布稳定 Release；版本号由 Cargo workspace 统一维护。

本项目基于 [MIT License](LICENSE) 开源，可自由使用、修改和分发，但需保留原始版权与协议声明。

## 文档与进度

- [资产仓库统一入口](https://github.com/Justwithyou/agent-dock-assets#readme)
- [Prompt 编写规范](https://github.com/Justwithyou/agent-dock-assets/blob/main/docs/prompt-spec.md)
- [Skill 编写规范](https://github.com/Justwithyou/agent-dock-assets/blob/main/docs/skill-spec.md)
- [评审与发布流程](https://github.com/Justwithyou/agent-dock-assets/blob/main/docs/review-and-release.md)
- [端到端示例](https://github.com/Justwithyou/agent-dock-assets/blob/main/docs/end-to-end-example.md)
- [产品设计](Product-Design.md)
- [开发任务与阶段验收](Development-Tasks.md)
- [测试约定](tests/README.md)

团队资产与本代码仓库分离维护；统一入口、负责人、评审和资产校验 CI 以资产仓库为准。
