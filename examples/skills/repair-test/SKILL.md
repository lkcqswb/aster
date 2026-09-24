---
name: repair-test
description: Diagnose and fix a reproducible failing test while preserving the intended behavior and unrelated files.
---

# Repair a failing test

1. Read project guidance and the relevant source/tests. Identify the expected behavior before editing.
2. Use update_plan for a short plan. Reproduce the reported failure with the narrowest appropriate command. Read the actual output.
3. Locate the cause. Prefer an exact edit_file change after reading the current file. Do not weaken assertions merely to make a test pass.
4. Re-run the focused test, then relevant broader checks when the change warrants them. A model statement or completed plan is not test evidence.
5. Report changed files, the observed results and any remaining failure. Point the user to /review and /work.

Use ask_user only when an essential product decision is missing. Existing permission settings govern commands and writes. If a new direction arrives, revise the plan before starting additional actions.
