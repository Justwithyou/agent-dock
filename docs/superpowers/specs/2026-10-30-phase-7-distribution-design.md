# 阶段 7：分发核心、清单与首批适配器设计

日期：2026-10-30

## 范围

实现 `Development-Tasks.md` 的 T7.1—T7.10：目标配置、适配器注册与接口、分发计划、冲突检测、原子复制、目标清单及测试。按用户补充要求，适配器覆盖 Claude Code 双类型、Cursor Prompt、Trae 双类型、Codex Skill 和 WorkBuddy 双类型。

CLI 与桌面分发界面属于阶段 8，本阶段不实现。保留现有基础 CI，不新增 GitHub Actions。完成实现后补充 GitHub README，更新任务状态并提交推送。

## 架构

采用“共享分发服务 + 纯适配器”：

- Core 定义 TargetProfile、适配器输入输出、DistributionPlan、文件状态、冲突和执行结果等领域类型。
- Adapters 按 `(kind, target)` 注册，只把 AssetSnapshot 转换为相对路径和文件内容，不读取 Git、不直接写文件、不决定版本。
- Infra 解析目标根目录，生成计划，检查路径和冲突，执行原子写入并维护清单。
- 后续 CLI 与桌面只调用同一分发服务，不各自实现分发算法。

不引入插件进程、数据库、常驻服务、符号链接分发、脚本执行或递归依赖安装。

## 目标配置

共享 `adock.yaml` 与本机 `adock.local.yaml` 继续沿用现有确定性合并规则，本机 TargetProfile 整项覆盖共享同名配置。

目标根目录支持绝对路径和 `~` 展开。相对根目录以资产工作区根目录为基准解析，不使用进程当前目录。目标名称为 `claude-code`、`cursor`、`trae`、`codex` 和 `workbuddy`，未配置的 TargetProfile 或未注册的 `(kind, target)` 返回稳定错误类别。TargetProfile 根目录分别指向工具的格式根（如 `.claude`、`.cursor`、`.trae`、`.agents`、`.codebuddy`），Trae 国际版与 CN 的用户级目录差异由本机配置处理，不进入适配器。

## 适配器契约与格式

适配器接收 AssetSnapshot，输出适配器版本及文件描述。文件描述包含仓内可移植相对路径和字节内容，以支持文本及二进制 Skill 资源。

### Claude Code

- Prompt 和 Skill 分别注册，可独立测试和扩展。
- 两类资产均转换为 `skills/<id>/SKILL.md`；Prompt 仅手动调用，frontmatter 包含 `name`、`description` 和 `disable-model-invocation: true`。
- Skill frontmatter 保留目标格式需要的 `name` 和 `description`，正文保持 Markdown。
- 多文件 Skill 的附属资源保留在 `skills/<id>/` 下的原相对结构中。
- 不输出旧式 `.claude/commands/*.md`。

### Cursor

- 本阶段实现 Prompt 一类，用于验证跨目标独立扩展。
- 输出 `skills/<id>/SKILL.md`，frontmatter 包含 `name`、`description` 和 `disable-model-invocation: true`。
- 不把 Prompt 转换为 Cursor Rules，不实现 `.mdc`。

### Trae

依据 Trae 官方[命令](https://docs.trae.cn/ide/slash-commands)与[技能](https://docs.trae.cn/ide/skills)文档，分别注册 Prompt 和 Skill：

- Prompt 输出 `commands/<id>.md`，frontmatter 包含 `name`、`description`，正文保留 Markdown 指令，不转换为 Rules 或 Skill。
- Skill 输出 `skills/<id>/SKILL.md`，frontmatter 包含 `name`、`description`，附属资源保留在 `skills/<id>/` 下的原相对结构中。
- 项目级根目录为 `.trae`；Trae CN 用户级根目录为 `~/.trae-cn`，国际版用户级根目录为 `~/.trae`，均由 TargetProfile 选择。
- 不输出超过官方命令目录嵌套层级的路径，本阶段统一使用 `commands/` 下的单文件命令。

### Codex

依据 OpenAI Codex 官方 [Agent Skills](https://developers.openai.com/codex/skills) 与 [Custom Prompts](https://developers.openai.com/codex/custom-prompts/) 文档：

- 本阶段实现 Skill 一类，输出 `skills/<id>/SKILL.md`，frontmatter 包含 `name`、`description`，正文为 Markdown，附属资源保留在 `skills/<id>/` 下。
- 项目级和用户级格式根统一使用 `.agents`，由 TargetProfile 指向项目内或用户主目录下的对应目录。
- 附属资源（包括 `scripts/`）只复制、不执行；不生成 `agents/openai.yaml` 等可选配置。
- Custom Prompts 已被官方标记为弃用，且仅支持 `~/.codex/prompts/` 用户级顶层 Markdown，不能随项目共享；本阶段不注册 Codex Prompt 适配器，也不把 Prompt 隐式转换为可自动触发的 Skill。

### WorkBuddy

依据腾讯 WorkBuddy 官方[项目配置](https://www.workbuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Project)、[斜杠命令](https://www.workbuddy.cn/docs/cli/slash-commands)与 [Skills](https://www.workbuddy.cn/docs/cli/skills) 文档，分别注册 Prompt 和 Skill：

- WorkBuddy 代码开发场景沿用 CodeBuddy Code 的 `.codebuddy` 目录契约，目标名为 `workbuddy`，格式根为 `.codebuddy`。
- Prompt 输出 `commands/<id>.md`，frontmatter 至少包含 `description`，正文保留 Markdown，通过 `/id` 显式调用。
- Skill 输出 `skills/<id>/SKILL.md`，frontmatter 包含 `name`、`description`，附属资源保留在 `skills/<id>/` 下的原相对结构中。
- 不生成 `allowed-tools`、Hooks、模型选择或内联 Shell 执行配置，不扩展到非代码开发场景的项目专家、连接器和资料库配置。

适配器版本使用稳定标识并写入清单；格式发生不兼容变化时必须显式升级版本。

## 计划与版本

DistributionPlan 包含 AssetRef、target、来源 RevisionSelector、来源 commit SHA、适配器版本、整体状态和逐文件计划项。每个计划项包含目标绝对路径、目标根目录内相对路径、SHA-256、内容、新增/更新/不变/冲突状态及冲突原因。

dry-run 与执行接收同一份计划，不重新选择 revision 或调用适配器生成另一份内容。执行前重新检查目标文件状态，发现计划失效时停止并报告冲突。

正式分发要求 AssetSnapshot 带有 commit SHA。HEAD、commit 和版本标签可直接读取 Git 快照，无需 checkout 或改变当前分支；未提交 WorkingTree 没有可追溯 commit，本阶段不允许作为正式分发来源。

## 路径与冲突

适配器输出路径必须为非空可移植相对路径，拒绝绝对路径、盘符或前缀、反斜杠、冒号、`.`、`..` 和空组件。目标路径必须留在 TargetProfile 根目录内，`.adock/` 保留给清单，适配器不得写入。已存在的符号链接路径组件不得用于分发。

冲突规则：

- 目标不存在：新增。
- 目标存在但不在清单：未纳管冲突，始终阻断，显式覆盖也不能绕过。
- 目标已纳管且当前校验和与清单一致：内容不同为更新，相同为不变。
- 目标已纳管但当前校验和与清单不同：本地修改冲突，默认阻断。
- 创建计划时显式允许覆盖后，已纳管本地修改可转为允许更新的计划项；未纳管冲突仍为阻断。
- 已纳管文件被删除后再次分发可按新增恢复，不视为覆盖。

本阶段只写适配器声明的文件，不删除目标侧其他文件。

## 执行与清单

执行器逐文件写入：在目标文件所在目录创建临时文件，完整写入后以原子重命名替换目标，不执行写入内容。单个文件失败不回滚已经成功的其他文件，结果明确区分已写入、不变、未处理和失败项，不能把部分失败报告为成功。

清单位于目标根目录 `.adock/distribution-manifest.json`，采用带格式版本的 JSON。清单逐文件记录 AssetRef、commit SHA、适配器版本、相对路径和 SHA-256，只作为目标侧纳管回执，不作为资产事实源。

每个文件写入成功后原子更新其清单项；不变文件只更新来源元数据，不重写资产文件。部分失败时，已成功文件已有清单记录，修复原因后重新生成计划即可安全重试。重复分发相同 commit 和内容不改变资产文件或清单。

## 错误处理

沿用结构化 AppError 和稳定错误码。未知适配器组合、无 commit 的分发来源、非法目标路径和阻塞冲突必须在写入前失败。文件系统错误包含目标路径和修复提示；执行结果保留逐项错误，不依赖操作系统原始文本判断错误类别。

## 测试与验收

- Core：目标配置、计划状态、覆盖规则和序列化模型单元测试。
- Adapters：Claude Code Prompt、单/多文件 Skill、Cursor Prompt、Trae Prompt/Skill、Codex Skill、WorkBuddy Prompt/Skill golden；Codex Prompt 等未注册组合和适配器路径契约测试。
- Infra：相对/绝对目标根目录、路径逃逸、未纳管冲突、纳管本地修改、显式覆盖、原子写入、幂等、清单追溯和部分失败重试测试。
- Git 集成：从历史标签快照直接生成计划，确认当前分支和工作区不变。
- 全量执行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 和 `cargo test --workspace`。

全部验收通过后，仅将阶段 7 标为完成，不勾选阶段 8。
