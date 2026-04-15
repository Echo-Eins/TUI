# P0 Remote Desktop Hardening

- [x] Read AGENTS.md, architecture docs, and current implementation.
- [x] Reject insecure remote-server credentials in config validation.
- [x] Remove all-zero tracked server secrets from repository config.
- [x] Fix incoming nonce state so forged ciphertext cannot advance replay state.
- [x] Replace Linux remote desktop capture/input stubs with real Linux backends.
- [x] Update docs for the actual remote desktop support and security flow.
- [x] Add reproducible Linux cross-check setup for native backend libraries.
- [x] Run `cargo test`, `cargo check --all-targets`, Linux `cross check`, and Linux `cross test`.

## Review Notes

- Console tab fixes are intentionally out of scope until remote desktop P0 is complete.
- Subagents were not used because this session only permits them when explicitly requested.
- Linux remote capture now uses `scrap` first and `grim`/`maim` fallback.
- Linux remote input now uses `enigo` first and `xdotool` fallback.
- Invalid capture regions now fail immediately instead of falling through to command fallback.
- `cargo test` passes with 67 tests.
- `cargo check --all-targets` passes on the local Windows target with existing warnings.
- `cross check --target x86_64-unknown-linux-gnu --all-targets` passes.
- `cross test --target x86_64-unknown-linux-gnu --all-targets` passes with 71 tests after `Cross.toml` installs link-time X11 dev libraries.

# Linux Console Repair

- [x] Read AGENTS.md and Linux Console design documents before implementation.
- [x] Fix Linux executor interrupt semantics and command-output state updates.
- [x] Fix command parsing and builtin handling for `cd`, `export`, `unset`, macros, and standard commands.
- [x] Remove unsafe command rewrites that change user intent on Linux.
- [x] Fix input cursor/history behavior for UTF-8 and long input.
- [x] Replace mojibake/garbled Console UI symbols with stable ASCII.
- [x] Fix syntax tokenization for command paths, redirections, and shell operators.
- [x] Add regression tests for Console command logic.
- [x] Run Windows and Linux check/test commands.

## Console Findings

- Linux `SIGINT` currently becomes a generic exit code because `ExitStatus::signal()` is ignored.
- Async completion updates operate on `active_block_id`, not the completed `block_id`, which is fragile when state changes around interrupts/failures.
- `find ...` is rewritten into `fd ...`, changing normal Linux command semantics.
- `cd "/path with spaces"` and quoted `export KEY="value with spaces"` are not parsed correctly.
- The UI and status badges contain mojibake symbols, making the Console tab look broken on normal terminals.
- Some cursor positions are set from byte lengths instead of character counts.
- Syntax highlighting marks executable paths such as `./script` as unknown commands and parses `2>` as an argument plus redirect.

## Console Review

- Linux executor now maps SIGINT to an interrupted block and other signals to shell-style `128 + signal` exit codes.
- Async output/completion/failure now finishes the exact command block by `block_id` instead of relying on the current active block.
- Builtins now use shell-word parsing for quoted paths and values; `export`, `unset`, `cd`, `cd -`, and parse errors produce explicit command blocks.
- `!!`, `!$`, and `sudo !!` now fail loudly when history is missing, and `sudo !!` avoids double-sudo.
- The Console no longer rewrites normal `find ...` commands into `fd` or `find . -name`; user command intent is preserved.
- Interactive command blocking now allows non-interactive invocations such as `python -c`, `node -e`, `bash -lc`, `bash script.sh`, and `sudo ls`, while still blocking real PTY/TUI workloads.
- Console UI status badges and major Console glyphs are ASCII-stable; long input is horizontally clipped around the cursor.
- Syntax highlighting now treats `./script` as a command token and `2>`/`2>>` as redirections.
- `cargo test` passes with 78 tests.
- `cargo check --all-targets` passes on the local Windows target with existing warnings.
- `cross check --target x86_64-unknown-linux-gnu --all-targets` passes with existing warnings.
- `cross test --target x86_64-unknown-linux-gnu --all-targets` passes with 84 tests after rerunning with a longer timeout.
- `git diff --check` is clean; Git only reports line-ending warnings from the Windows worktree.

# Native Linux Link Fix

- [x] Diagnose native Linux `rust-lld: unable to find library -lxdo`.
- [x] Remove mandatory Linux `enigo`/`libxdo` linkage from the default build.
- [x] Keep Windows `enigo` backend intact.
- [x] Move Linux input injection to a runtime `xdotool` backend.
- [x] Update Linux build/runtime docs and Cross image dependencies.
- [ ] Re-run local and Linux cross verification after the link fix.

## Native Linux Link Finding

- `libxdo` was pulled in by the Linux `enigo` dependency at link time.
- The code already had an `xdotool` runtime fallback, but fallback code cannot help if the binary cannot link.
- Default Linux builds should not require `libxdo-dev`; only runtime input injection should require `xdotool`.
