# AgentDock 测试约定

## 分层

- Core：领域模型、解析、变量替换、路径边界和快照 diff 使用单元测试。
- Infra：文件系统发现、初始化、配置与 Schema 契约使用临时目录测试。
- Git 与 CLI：使用真实临时 Git 仓库；CLI 使用 `assert_cmd` 驱动真实二进制，不模拟版本算法。
- 桌面：Rust 命令直接复用共享 Core/Infra/Adapters；桌面—CLI 交叉测试验证标签、revision、错误类别和恢复变更。
- 适配器与桌面安装：分别在 `golden/adapters/` 和 `desktop-contract/` 建立契约，不在单元测试中要求真实外部 Agent 成功执行。

## 数据与目录

- `fixtures/` 存放跨测试复用的最小 Prompt、多文件 Skill 和配置；仅在至少两处复用时外置，现有内联构造无需为目录统一而迁移。
- `golden/adapters/` 存放同一份资产在各目标中的预期导出结果；Skill golden 必须覆盖入口文件与附属资源。
- `desktop-contract/` 存放后续桌面安装与跨平台契约数据，不包含机器相关绝对路径。
- 临时 Git 仓库必须在测试中创建唯一用户身份并自动清理，不依赖开发机全局 Git 配置或远端服务。

## 断言与 golden 更新

- 同时覆盖成功路径和必要错误类别；错误断言保留稳定字段，避免绑定完整可变文案。
- 路径、commit SHA 和临时目录不得直接写死为平台相关值；比较前使用明确的相对路径或解析结果。
- golden 只能因目标格式契约或预期转换规则变化而更新。更新时必须审查完整差异，不能用新输出覆盖失败的未知行为。
- 每个阶段交付前运行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 和 `cargo test --workspace`。
