# AgentDock 工程基线决策

查询与确认日期：2026-10-30。命名检索只是发布前初检，不构成法律意见或商标清查。

## 命名与标识

代码名继续使用 AgentDock，命令行标识继续使用 `adock`。公开发布任何包之前，为包名增加团队或组织前缀，并重新检查目标注册中心。

| 名称 | GitHub 初检 | npm 初检 | PyPI 初检 |
|---|---|---|---|
| AgentDock / agentdock | [AgentDock/AgentDock](https://github.com/AgentDock/AgentDock) 已存在；仓库名按所有者隔离 | [agentdock](https://registry.npmjs.org/agentdock) 已发布 | [agentdock](https://pypi.org/pypi/agentdock/json) 已发布 |
| agent-dock | [Trungsherlock/agent-dock](https://github.com/Trungsherlock/agent-dock) 已存在 | [agent-dock](https://registry.npmjs.org/agent-dock) 已撤版，发布前仍需复查 | [agent-dock JSON API](https://pypi.org/pypi/agent-dock/json) 未返回已发布项目 |
| adock | [adock27/adock](https://github.com/adock27/adock) 已存在；本地 CLI 名不依赖全局包名 | [adock](https://registry.npmjs.org/adock) 未返回已发布包 | [adock JSON API](https://pypi.org/pypi/adock/json) 未返回已发布项目 |

[WIPO Brand Database 初检](https://branddb.wipo.int/en/quicksearch?by=brandName&v=AgentDock) 受人机验证限制，不能据此认定无商标冲突。正式分发前仍需由发布责任人完成适用地区的商标检查。

## 桌面平台与验收矩阵

| 项目 | 基线 |
|---|---|
| 默认平台 | Windows 11 x64 干净环境 |
| 补充平台 | 团队仍使用 Windows 10 时，增加 Windows 10 22H2 x64 |
| 必需运行时 | WebView2 与 Git；不依赖 Node.js 或 Python |
| 安装与启动 | 使用阶段 10 产出物安装、启动和打开同一资产仓 |
| 生命周期 | 覆盖安装、启动、升级和卸载 |
| 环境分支 | 覆盖 WebView2/Git 已具备与缺失时的预期行为 |
| 路径与语言 | 覆盖中文路径、长路径及无 Node.js/Python 环境 |

Tauri 的 Windows 环境与 WebView2 前置条件以 [Tauri 官方 Prerequisites](https://v2.tauri.app/start/prerequisites/) 为准。本表是测试矩阵，不把当前开发机通过等同于干净环境验收完成。

## 第一适配目标：Claude Code

依据 [Claude Code Skills 官方文档](https://code.claude.com/docs/en/skills)：

- 自定义 commands 已并入 Skills；旧式 `.claude/commands/*.md` 仍用于兼容既有命令。
- Skill 使用 `.claude/skills/<name>/SKILL.md`，由 YAML frontmatter 和 Markdown 正文组成，附属资源放在同一 Skill 目录内。
- Prompt 与 Skill 均须经过显式转换；不能把 AgentDock 的 frontmatter 或路径不做处理地当成目标格式。

## 第二适配目标：Cursor

依据 [Cursor Skills 官方文档](https://cursor.com/docs/skills)：

- Skill 可位于 `.cursor/skills/<name>/` 或 `.agents/skills/<name>/`，入口为 `SKILL.md`，附属资源随目录保留。
- 显式 Prompt 可转换为仅手动调用的 Skill：YAML frontmatter 至少包含 `name`、`description`，并设置 `disable-model-invocation: true`，正文保留 Markdown，以 `/name` 调用。
- 旧 commands 页面已并入 Skills，不再为新 Prompt 依赖缺少现行契约的 `.cursor/commands`。
- [Cursor Rules](https://cursor.com/docs/rules) 使用 `.cursor/rules/*.mdc`，负责持久或条件注入上下文，不作为显式 Prompt 的替代品。

因此 MVP 的两个适配目标确认为 Claude Code 与 Cursor；Codex 不进入本期范围。具体导出、预览和写回仍在阶段 7 实现。
