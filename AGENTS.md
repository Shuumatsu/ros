## Engineering standards

- **No patch-style fixes.** Do not paper over a problem at the symptom site. Fix
  the root cause, even when that means more work or a larger change.
- **Always apply best practices.** Extra effort and architectural changes are
  acceptable and expected when they lead to a cleaner, more correct design.
- **Don't repeat yourself (DRY).** No duplicated logic. Extract shared behavior
  into a single source of truth.
- **No split-brain.** A given piece of state, logic, or knowledge must live in
  exactly one place. Never let two parts of the system independently track or
  decide the same thing.

- Strict Modularity: Enforce logical file splitting and absolute separation of concerns. Do not output monolithic, single-file solutions.

- High Reusability: Design highly modular, decoupled components and modular implementations aimed at maximum reusability across the codebase.

---

- git history does not make sense. focus on current codes