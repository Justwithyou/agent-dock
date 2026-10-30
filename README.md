# AgentDock

AgentDock 是面向研发团队的 AI 能力资产管理工具：以 Git 为唯一事实源，统一管理 Prompt 与 Skill，并通过同一 Rust Core 为 `adock` CLI 和 Tauri 桌面端提供版本管理与安全分发能力。

## 当前能力

- Prompt / Skill 工作区初始化、发现、列表、详情与校验。
- Prompt 变量声明、默认值、必填检查和显式渲染。
- 多文件 Skill 资源读取、依赖声明检查与路径边界校验。
- 基于 Git 的历史、双版本 diff、资产标签、`prod` 通道和单资产回滚。
- CLI 与桌面端共享同一核心，不维护第二套资产或版本算法。
- 分发计划、SHA-256 校验、冲突检测、原子写入、失败重试和目标侧清单。
- Claude Code、Cursor、Trae、Codex、WorkBuddy 首批格式适配器。

> 当前已完成开发阶段 0—7。CLI 与桌面端的分发向导属于阶段 8，尚未开放；现阶段分发能力由共享 Rust 服务提供。

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
adock rollback prompt code-review --to adock/prompt/code-review/v1.0.0
adock rollback prompt code-review --to adock/prompt/code-review/v1.0.0 --yes
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

## 文档与进度

- [产品设计](Product-Design.md)
- [开发任务与阶段验收](Development-Tasks.md)
- [阶段 7 分发设计](docs/superpowers/specs/2026-10-30-phase-7-distribution-design.md)
- [测试约定](tests/README.md)

阶段 8 将接入 CLI 与桌面端分发闭环；后续统一入口、系统加固和发布验收以开发任务清单为准。
