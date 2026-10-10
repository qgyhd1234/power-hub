# Issue Tracker：本地 Markdown

本仓库的 issue 和 spec 以 markdown 文件形式存放在 `.scratch/` 目录。

## 约定

- 每个功能一个目录：`.scratch/<feature-slug>/`
- spec 文件为 `.scratch/<feature-slug>/spec.md`
- 实现 issue 每张票一个文件：`.scratch/<feature-slug>/issues/<NN>-<slug>.md`，从 `01` 开始编号，禁止把所有 ticket 合并在单个文件里
- 分诊状态记录在 issue 文件顶部的 `Status:` 行（角色字符串见 `triage-labels.md`）
- 评论和讨论历史追加到文件底部的 `## Comments` 标题下

## 当技能说"发布到 issue tracker"

在 `.scratch/<feature-slug>/` 下创建新文件（必要时创建目录）。

## 当技能说"获取相关 ticket"

读取引用路径处的文件。用户通常会直接给出路径或 issue 编号。

## Wayfinder 操作

由 `/wayfinder` 使用。**地图（map）**是一个文件，每张票一个**子文件**。

- **Map**：`.scratch/<effort>/map.md`（正文含 Notes / Decisions-so-far / Fog）。
- **子票**：`.scratch/<effort>/issues/NN-<slug>.md`，从 `01` 开始编号，正文写问题。`Type:` 行记录票类型（`research`/`prototype`/`grilling`/`task`）；`Status:` 行记录 `claimed`/`resolved`。
- **阻塞**：顶部附近的 `Blocked by: NN, NN` 行。当它列出的每个文件都是 `resolved` 时，该票解除阻塞。
- **Frontier**：扫描 `.scratch/<effort>/issues/` 中打开、未阻塞、未认领的文件；编号最小者优先。
- **认领**：先设置 `Status: claimed` 并保存，再做任何工作。
- **解决**：在 `## Answer` 标题下追加答案，设置 `Status: resolved`，然后把上下文指针（要点 + 链接）追加到 `map.md` 的 Decisions-so-far。
