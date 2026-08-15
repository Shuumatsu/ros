## Engineering standards

- **No patch-style fixes.** Do not paper over a problem at the symptom site. Fix the root cause, even when that means more work or a larger change.
- **Always apply best practices.** Extra effort and architectural changes are acceptable and expected when they lead to a cleaner, more correct design.
- **Don't repeat yourself (DRY).** No duplicated logic. Extract shared behavior into a single source of truth.
- **No split-brain.** A given piece of state, logic, or knowledge must live in exactly one place. Never let two parts of the system independently track or decide the same thing.

- Strict Modularity: Enforce logical file splitting and absolute separation of concerns. Do not output monolithic, single-file solutions.

- High Reusability: Design highly modular, decoupled components and modular implementations aimed at maximum reusability across the codebase.

---

- git history does not make sense. focus on current codes
- be simple on git commit messages
- write comments in a conservative and restrained manner

---

For comments: 你正在编辑一份长期使用的 comment，而不是生成讨论记录。

讨论内容仅用于判断事实是否正确。不要把讨论过程、错误假设、 反驳过程或纠正痕迹写进最终 doc。

编辑要求：
1. 最终文本必须像独立的 official document，读者不需要知道此前发生过讨论。
2. 只保留最终确认的知识结论。
3. 优先使用肯定式陈述句。
4. 如果结论由“A 是错误的，正确的是 B”得出，只写 B。
5. 不要写“不是 A”“容易误以为 A”“与此前说法不同”等讨论性表述。
6. 除了事实纠正之外，尽量保持原笔记的结构、信息密度和语气。
7. 只有当否定本身构成重要知识边界时，才保留否定句。
8. 只输出修改后的 comment。

---

