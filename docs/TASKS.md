# Project tasks with 弄玉

Press **F8**, select **Run a project check or build** in her F1 menu, or use `/tasks [filter]`. Discovery reads local files; opening the picker does not execute anything or call the model. Type or paste to filter, use Up/Down to choose, **Tab** to inspect the source, timeout and exact command, and **Enter** to run. Escape returns to the list or closes it. Your composer draft stays intact.

While inspecting, 弄玉 uses her reading pose. After execution begins, her work card follows the command and **F4 /output** shows streaming output. A nonzero exit, timeout or stop remains visible as evidence needing attention. A successful command means that command exited successfully; it does not establish that the entire project is correct.

`/task NAME` runs a named task directly. Both routes use the same shell permissions, process cleanup, output capture and plan-mode restrictions as `/run`. With the default `ask` policy you review the actual command before it starts. With `allow`, Enter starts it immediately. Plan mode blocks execution. No model request is made. A task cannot start while another turn or decision is pending; you can still inspect the list.

## Configuration and discovery

If `.aster/tasks.json` exists, it replaces automatic discovery. For example:

```json
{
  "tasks": [
    {
      "name": "test",
      "description": "Run the project test suite",
      "command": "cargo test",
      "timeout_secs": 120
    },
    {
      "name": "lint",
      "description": "Reject Rust warnings",
      "command": "cargo clippy --all-targets -- -D warnings"
    }
  ]
}
```

A tracked starter is in `examples/tasks.json`. The Aster repository ignores its own private `.aster` directory; copy the example into the project where you want to use it.

Without explicit configuration, `Cargo.toml` provides `cargo:test`, `cargo:check` and `cargo:build`. String entries in `package.json` scripts become tasks using the detected lockfile's runner: pnpm, yarn, bun, then npm as the fallback. Multiple lockfiles use that order. Commands use `RUNNER run SCRIPT`; package scripts may themselves run arbitrary project code, which is why the usual shell permission applies.

Limits: 32 tasks, 64 KB per source file, unique names up to 64 ASCII letters/numbers/hyphens/underscores/colons starting with a letter or number, descriptions up to 240 bytes, commands up to 4,000 bytes and timeouts from 1 to 120 seconds (default 120). Source files must be regular files inside the project; symlink sources and unknown configuration fields are rejected. Unsupported package-script names are skipped. The task is resolved again when run so the approval uses current configuration, not an old picker snapshot.

## Automation outcomes

```sh
aster --project /path/to/project --permissions allow --prompt '/task test'
```

Headless tasks exit nonzero for failure, timeout, stop, denied permission or an unknown name. More generally, headless turns exit nonzero when recorded checks still fail or have become stale after file-tool edits, even if the model finished its reply. A successful rerun of the same check can resolve a prior failure. A conversational reply with no recorded checks can exit successfully; absence of checks is not presented as verified work. Headless mode cannot ask for approval or answer a model question interactively.
