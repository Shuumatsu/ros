## Engineering standards

- **No band-aid fixes.** Do not paper over problems by addressing only their symptoms. Fix the root cause, even when doing so requires more work or broader changes.
- **Always follow best practices.** Additional effort and architectural changes are acceptable and expected when they produce a cleaner, more correct design.
- **Don't repeat yourself (DRY).** Do not duplicate logic. Extract shared behavior into a single source of truth.
- **Avoid split-brain designs.** Each piece of state, logic, or knowledge must live in exactly one place. Never allow two parts of the system to track or determine the same thing independently.
- **Strict modularity.** Divide code logically across files and maintain a complete separation of concerns. Do not produce monolithic, single-file solutions.
- **High reusability.** Design highly modular, decoupled components and implementations to maximize reusability across the codebase.

---

- Git history is not meaningful. Focus on the current codebase.
- Keep Git commit messages simple.
- Do not add comments indiscriminately. Omit unnecessary comments, and write necessary ones conservatively and sparingly. Prefer expressing facts through code over explaining code with comments.
- The repository contains legacy comments. Do not rely on them, and clean them up when they are incorrect.
- Write Git commit messages on a single line.

- The project is currently at an early stage, so compatibility is not required.
- Complete refactoring is allowed and does not need to be limited to incremental changes.
- Avoid ad hoc solutions.

---

When editing comments or documentation, treat them as long-lived material rather than a record of discussion.

Use the discussion only to determine which facts are correct. Do not include the discussion process, incorrect assumptions, counterarguments, or traces of corrections in the final comments or documentation.

Editing requirements:
1. The final text must read as standalone official documentation or comments. Readers should not need any knowledge of the preceding discussion.
2. Include only conclusions that have been confirmed.
3. Prefer affirmative declarative statements.
4. If a conclusion was reached by determining that “A is incorrect; B is correct,” write only B.
5. Do not include discussion-oriented phrases such as “not A,” “it is easy to assume A,” or “unlike the previous explanation.”
6. Except for factual corrections, preserve the original structure, information density, and tone as closely as possible.
7. Retain negative statements only when the negation itself establishes an important knowledge boundary.
8. Output only the revised comments or documentation.

---

This project aims to implement support for the RISC-V64 architecture, covering bare-metal booting, privilege-level switching and system calls, preemptive task scheduling, process lifecycle management, virtual memory with kernel/user address space isolation, and a simple file system.
I do not wish to get bogged down in hardware intricacies or edge cases—such as security vulnerability—which I view as mere noise in the learning process.
