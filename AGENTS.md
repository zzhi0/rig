# 开发协作

- Rig 是本地运行、Web UI 优先的个人 Agent harness；内核保持任务中立。当前范围与边界见 [架构](docs/architecture.md)，命令与验收见 [开发指南](docs/development.md)。
- 核心设计、数据模型、模块边界、重要依赖和阶段范围的变更，先解释机制、取舍与影响，经项目维护者确认后实施；已确认范围内的常规实现自主推进。阶段结束时讲解核心实现与验证结果。
- 内核拥有业务事实和状态变化；展示数据不绑定 UI；客户端数据与状态模块不依赖组件。UI 负责渲染和局部交互状态。避免为尚未实现的能力预建抽象。
- 仅支持 macOS 和 Linux，永不支持 Windows。文档使用中文；源代码、代码注释和 Rustdoc 使用英文。设计文档只保留当前有效结论与必要理由，随实现更新，不保存讨论草稿、临时计划或 ADR 流水记录。
- 不默认添加防御代码、兼容层或回退分支；发现需要时先说明原因并取得明确确认。
- 使用独立 Git worktree 开发，通过 PR 审查并合入 `main`。允许主题一致的较大 PR。关键设计确认后明确职责、接口与验收标准，再将独立工作委派给不同 worktree 中的子 Agent；最终负责集成验证。
- 修改后运行与范围相称的检查，执行必要的工程检查；测试关注行为和边界，不为低影响可逆改动添加照抄实现的测试。交付时说明变更、验证和实际限制。
- 提交遵循 Conventional Commits，不添加 `Co-Authored-By: Codex ...`。不提交 `docs/superpowers/` 下的任何产物，即使它们未被忽略。
- Superpowers 插件禁用。仅在维护者按名称明确调用时使用保留的 `brainstorming`、`writing-plans`、`systematic-debugging`、`verification-before-completion` 技能，禁止自动串联。
