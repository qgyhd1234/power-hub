# 领域文档

工程技能在探索代码库时应如何消费本仓库的领域文档。

## 探索前先读这些

- 仓库根目录的 **`GLOSSARY.md`**
- **`docs/adr/`**：阅读与即将工作的领域相关的 ADR

如果这些文件不存在，**静默继续**。不要提示它们缺失，也不要建议提前创建。`/domain-modeling` 技能（通过 `/grill-with-docs` 和 `/improve-codebase-architecture` 进入）会在术语或决策实际敲定时惰性创建它们。

## 文件结构

单上下文仓库：

```
/
├── GLOSSARY.md
├── docs/adr/
│   ├── 0001-event-sourced-orders.md
│   └── 0002-postgres-for-write-model.md
└── src/
```

## 使用词汇表的词汇

当输出涉及领域概念（issue 标题、重构提案、假设、测试名）时，使用 `GLOSSARY.md` 中定义的术语。不要漂移到词汇表明确避免的同义词。

如果你需要的概念还不在词汇表中，这是一个信号：要么你在发明项目不用的语言（请重新考虑），要么存在真实缺口（记录下来交给 `/domain-modeling`）。

## 标记 ADR 冲突

如果你的输出与现有 ADR 矛盾，明确指出而不是默默覆盖：

> _与 ADR-0007（event-sourced orders）矛盾，但值得重新讨论，因为……_
