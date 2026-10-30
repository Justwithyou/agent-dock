# 阶段 0/1/2 待补项完成计划

## Context

阶段 3—6 已完成，但阶段 0—2 仍存在工程证据和必要测试缺口：命名/平台/适配器结论未归档，独立资产仓与 CI 未建立，测试约定和 golden 目录不完整，Schema 未覆盖真实模型契约，Skill 资源路径缺少越界校验。目标是一次性补齐这些缺项，使阶段 0—2 能按统一 DoD 标为完成，不进入阶段 7 的适配器实现。

默认采用本地独立 `agent-dock-assets/` 资产仓、Windows 11 x64 首期验收基线（团队使用 Windows 10 时补测 22H2 x64）、Claude Code + Cursor 两个适配目标。不配置远端仓库、凭证，不提交或推送代码。

## 实施步骤

### 1. 归档工程决策与测试约定

- 新增 `docs/engineering-baseline.md`：记录命名初检的 GitHub/npm/PyPI 一手链接和冲突结果、WIPO 初检限制及“公开包名增加团队前缀”的处理；记录 Windows 最小验收矩阵；记录 Claude Code 与 Cursor 官方目录、格式和能力边界，附查询日期及官方 URL。
- 新增 `tests/README.md`：约定 Core 单元测试、临时 Git 集成测试、CLI `assert_cmd` 测试、桌面—Core 契约测试，以及 fixtures/golden 的放置和更新规则。
- 建立 `tests/fixtures/`、`tests/golden/adapters/`、`tests/desktop-contract/` 目录骨架；保留现有内联测试，不做无关迁移。

### 2. 补齐模型与 JSON Schema 契约

- 修改 `crates/infra/src/init.rs` 中 `PROMPT_SCHEMA` 和 `SKILL_SCHEMA`，覆盖平铺 metadata、body、各类型的具体 `spec`、必填字段及未知字段约束；Schema 校验对象明确为“平铺 metadata + 具体 spec + body”，不是带 `type/value` 的完整 `Asset`。
- 在 `crates/infra/Cargo.toml` 增加 `jsonschema` 开发依赖并更新 `Cargo.lock`。
- 复用 `WorkspaceInitializer`、`parse_asset`、serde 和现有临时目录测试：验证初始化 Prompt/多文件 Skill 示例通过对应 Schema；增加缺必填字段、错误 spec、未知顶层字段等反例，确保模型和 Schema 双向约束不失配。

### 3. 补齐 Skill 路径边界校验

- 修改 `crates/core/src/validation.rs`，复用 `validate_relative_asset_path` 校验 Skill 的 entrypoint 和所有 `source_files`。
- 在拒绝绝对路径、盘符/前缀、反斜杠和 `..` 后，再按资产 ID 得到的 `skills/<id>/` 根目录做词法边界检查，防止资源或入口指向其他 Skill/工作区路径；将通用路径错误文案从仅 entrypoint 扩展到所有 Skill 路径。
- 新增表驱动测试，覆盖合法 POSIX 路径、缺失 entrypoint、POSIX/Windows 绝对路径、反斜杠、`..`、跨 Skill 资源逃逸，并验证合法多文件 Skill 不误报。

### 4. 建立独立资产仓与 CI

- 在 Schema 和校验完成后，使用 `cargo run -p adock-cli -- --workspace agent-dock-assets init` 复用 `WorkspaceInitializer` 生成独立资产仓的配置、README、Schemas、示例 Prompt/多文件 Skill 和忽略规则，再执行 `git -C agent-dock-assets init`；不创建提交。
- 修改根 `.gitignore` 忽略 `/agent-dock-assets/`，明确工具代码与资产内容分仓。
- 新增 `.github/workflows/ci.yml`：Windows 环境安装 Rust 1.85、rustfmt 和 Clippy，运行格式检查、全目标 Clippy、workspace 测试；再用 CLI 初始化并校验临时资产工作区，覆盖资产验证和现有桌面 Rust 契约测试，不依赖私有资产仓凭证。

### 5. 更新任务状态

- 所有交付和验收通过后，更新 `Development-Tasks.md` 的实施进度，将 T0.1—T0.7、T1.1—T1.9、T2.1—T2.7 标为完成，并勾选阶段 0—2 的验收项。
- 若某项真实验证未通过，保留部分完成状态并记录实际缺口，不提前勾选。

## 验证

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo run -p adock-cli -- --workspace agent-dock-assets validate`
5. 检查 `agent-dock-assets/.git` 独立存在，且配置、Schemas、README、示例 Prompt/多文件 Skill、`.gitignore` 完整。
6. 核对 CI YAML 与 `docs/engineering-baseline.md` 的链接、平台矩阵和测试约定；确认阶段 0—2 验收项均有对应实现或记录。
