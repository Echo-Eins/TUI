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
- [x] Re-run local and Linux cross verification after the link fix.

## Native Linux Link Finding

- `libxdo` was pulled in by the Linux `enigo` dependency at link time.
- The code already had an `xdotool` runtime fallback, but fallback code cannot help if the binary cannot link.
- Default Linux builds should not require `libxdo-dev`; only runtime input injection should require `xdotool`.
- `cargo tree --target x86_64-unknown-linux-gnu` no longer includes `enigo` or `xdo`.
- `cargo test` passes with 78 tests.
- `cargo check --all-targets` passes on the local Windows target with existing warnings.
- `cross build --target x86_64-unknown-linux-gnu --release --bin TUI` passes without `libxdo-dev`.
- `cross check --target x86_64-unknown-linux-gnu --all-targets` passes with existing warnings.
- `cross test --target x86_64-unknown-linux-gnu --all-targets` passes with 84 tests.

# High-Severity Linux/TUI Reliability Fixes

- [x] Preserve padded `scrap` frame stride and add a regression test.
- [x] Only create a default TUI config when the config file is missing; never overwrite malformed or unreadable user config.
- [x] Run Linux monitor collection on Tokio's blocking pool while preserving monitor state.
- [x] Replace sleep-based Linux CPU sampling with deltas between refresh cycles.
- [x] Add bounded execution with timeout and process termination for Linux monitor commands.
- [x] Apply configured command timeout to Ollama polling and actions.
- [x] Run formatting, tests, checks, and targeted lint verification.
- [x] Launch the TUI in a PTY and inspect runtime threads and CPU behavior.

## High-Severity Review

- Added timeout-controlled process execution that terminates the full Unix process group and drains stdout/stderr without pipe deadlocks.
- Linux monitor collection no longer blocks Tokio worker threads; the regression test verifies timer progress on a single-worker runtime and preserves monitor state across collections.
- Linux CPU usage now uses `/proc/stat` deltas between refresh cycles rather than sleeping twice per collection.
- Verification passed: `cargo fmt --all`, `cargo test --all-targets` (128 tests), `cargo check --all-targets`, targeted Clippy checks for `await_holding_lock` and `large_futures`, `cargo build --bin TUI`, and `git diff --check`.
- A 15-second PTY run remained stable at 27 threads across four samples, left no child processes, and left no TUI process after termination. A separate interactive PTY run also restored the terminal and exited cleanly on Ctrl+C.

# Linux NVIDIA and Storage Redesign

- [x] Parse `nvidia-smi` CSV per GPU row with quoted-field and unsupported-value handling.
- [x] Use the explicit NVIDIA GPU index and UUID instead of deriving an index from PCI bus IDs.
- [x] Scope compute, graphics, and `pmon` process data to the selected GPU.
- [x] Exclude zram, loop, RAM, device-mapper, md, NBD, and sysfs-virtual devices from physical disks.
- [x] Resolve filesystem devices through `lsblk` parents and sysfs slaves for partitions, LVM, dm-crypt, and md stacks.
- [x] Deduplicate Btrfs subvolume mounts into one filesystem volume.
- [x] Separate physical capacity from mounted-filesystem capacity and usage.
- [x] Replace fixed-width Disk text rows with adaptive tables and a unified I/O dashboard.
- [x] Add backend, parser, aggregation, graph-title, and render regression tests.
- [x] Verify the redesigned Disk tab against the host's real NVMe/Btrfs topology.

## NVIDIA and Storage Review

- The host now reports one physical `nvme0n1`; the 7.69 GiB false SSD was `zram0` and is no longer classified as storage.
- The Btrfs root filesystem is represented once with nine mount points instead of nine duplicate capacity rows; EFI remains a separate filesystem.
- The physical disk card reports 953.87 GB device capacity and 502.57 GB used across 945.87 GB of mounted filesystems.
- Disk queue depth now uses the kernel weighted-I/O-time average and no longer takes the maximum with an instantaneous in-flight request count.
- Verification passed: `cargo fmt --all`, `cargo test --all-targets` (139 tests), `cargo check --all-targets`, targeted Clippy checks, `cargo build --bin TUI`, `git diff --check`, narrow and 180-column PTY rendering, and a 15-second runtime thread sample stable at 27 threads.
- Live NVIDIA data could not be validated on this host because `nvidia-smi` cannot communicate with the installed driver; synthetic tests cover multiple GPUs, quoted CSV, units, missing values, and truncated rows.

# Console Extension Platform, Math Modulator, and Mini-Games

## Current Status

- [x] First implementation milestone completed: Network graph bug fix plus Console extension platform foundation.
- [x] Second implementation milestone completed: first useful built-in math module with `:base`, `:calc`, and shared parser/AST.
- [x] Third implementation milestone completed: exact-first math output, `-num`, `-mb`, formula rendering, and `for <var>` formula target support.
- [x] Correction pass: fix relation-level `for <var>`, symbolic parameters, exact trigonometric families, and Math Block root rendering.
- [x] Fourth implementation milestone: finish formula command surface and implement the first bounded fallback `:plot` math modulator.
- [x] Fifth implementation milestone: promote `:plot` to a typed Console output rendered by ratatui widgets, with ASCII fallback kept only as fallback/history output.
- [x] Sixth implementation milestone: add trig Math Block unit-circle visuals, explicit plot axes, real `ConsoleSession` runtime wiring, and interactive plot zoom/pan.
- [x] Seventh implementation milestone: make ratatui the production renderer for Console visual blocks, fix Tab ghost-completion, and close exact trig-power solver gaps.
- [x] Keep the Linux Console repair and native Linux link fix as the quality baseline.
- [x] Preserve normal shell behavior. Console extensions must not steal ordinary shell commands unexpectedly.
- [x] Keep secondary functionality lazy, bounded, and inactive until requested.
- [x] Treat every module as production code: small APIs, clear ownership, no unbounded work, no UI overflow, deterministic tests.
- [x] For this milestone, run static checks only: `cargo check --all-targets` and Linux `cross check --target x86_64-unknown-linux-gnu --all-targets`.

## Product Direction

- [ ] Build Console as a terminal-first workspace, not only a shell transcript.
- [ ] Do not create a separate Games tab in the first phase; games belong inside Console to preserve the terminal-toys vibe.
- [ ] Treat games as hidden/optional Console apps launched from commands, not as a standalone product section.
- [x] Keep the default shell path sacred: unknown commands continue to go to the real shell.
- [x] Add secondary features through explicit Console extension commands.
- [x] Prefer an explicit internal prefix such as `:` for extension commands: `:calc`, `:plot`, `:base`, `:play`, `:mods`.
- [ ] Consider unprefixed aliases only after a config flag exists and collisions with real shell commands are handled.
- [ ] Make extensions feel native to the Console tab: fast keyboard flow, compact output blocks, clear errors, no modal clutter.
- [ ] Match the visual quality of the Network tab: terminal aesthetic, modern spacing/color, readable borders, responsive layouts.
- [x] Keep normal command blocks and extension output blocks visually compatible so Console history remains coherent.

## Architecture Requirements

- [x] Introduce a small Console extension layer instead of hardcoding every feature directly into `AppState`.
- [x] Add a `ConsoleCommandRouter` responsible for detecting internal commands and preserving shell passthrough.
- [x] Add a `ConsoleExtensionRegistry` that owns built-in extension metadata and lookup.
- [x] Define `ExtensionMetadata` with id, title, description, version, kind, commands, permissions, and tags.
- [x] Define `ConsoleCommandSpec` so help, completion, validation, tags, and examples are generated from one source.
- [x] Define a `ConsoleExtension` API for one-shot commands such as `:base`, `:hash`, `:formula`, and `:calc`.
- [x] Define a `ConsoleSession` API for interactive modules such as games and graph explorers.
- [x] Define a `ConsoleContext` carrying theme, terminal size, config, current directory, environment snapshot, and permission policy.
- [x] Define a `ConsoleResult` model with explicit variants: text, table, formula, plot, canvas, interactive session, structured error.
- [x] Define `ExtensionKind` as a future-proof model: built-in, script, external process, and later Wasm.
- [x] Route input as: `:` prefix goes to Console extensions; everything else goes to the normal shell executor unless a user-enabled alias maps it.
- [x] Keep command parsing separate from rendering and execution.
- [x] Keep math parsing/evaluation separate from Console UI.
- [ ] Keep game state/update/render code separate from shell command state.
- [ ] Add a `GameRuntime` thin wrapper over `ConsoleSession` for game-specific lifecycle, scores, summaries, and commands.
- [ ] Add a `GameRegistry` so adding a built-in game is registration, not a new hardcoded branch in Console.
- [ ] Define a `ConsoleGame` API with id, title, help, tags, config schema, and `new_session`.
- [ ] Keep plugin loading separate from built-in modules so security rules remain obvious.
- [x] Do not add global mutable state for modules.
- [ ] Avoid module-specific behavior in the generic command history path unless it is explicitly part of the API.
- [x] Wire `ConsoleSession` into Console command blocks, key dispatch, ticking, rendering, termination, and history summaries before adding games.

## Performance Requirements

- [x] Idle Console must not tick games, plots, calculators, or plugins.
- [x] Inactive modules must use zero periodic CPU.
- [x] Interactive modules must have a capped tick rate per module.
- [ ] Games should update at fixed logical ticks, not as fast as the render loop can run.
- [ ] Graph sampling must be bounded by terminal width and an explicit sample cap.
- [x] Plot data must be cached and recomputed only when expression, domain, sampling settings, or widget size changes.
- [ ] Formula layout must cache parsed/rendered AST output where possible.
- [ ] Calculator expressions must compile or parse once per submitted command where possible.
- [ ] Avoid per-frame heap allocations in game hot paths and plot render hot paths.
- [ ] Cap output block length and history growth for generated module output.
- [ ] Never block the UI/render loop on plugin execution, expensive math, filesystem scans, or external commands.
- [ ] Add regression tests for every resource limit that protects CPU, memory, and UI responsiveness.

## Core Commands

- [x] `:help` prints shell-safe Console extension help.
- [x] `:mods` lists available built-in and user modules.
- [ ] `:mod info <name>` shows manifest, commands, permissions, and status.
- [ ] `:mod enable <name>` enables a disabled optional module where allowed.
- [ ] `:mod disable <name>` disables an optional module where allowed.
- [ ] `:mod reload <name>` reloads user modules without restarting the app.
- [x] `:calc <expr>` evaluates engineering calculator expressions.
- [x] `:calc <expr> -num` or `:calc <expr> --num` explicitly requests numeric approximation.
- [x] `:calc <expr> -mb` opens a rich Math Block instead of compact Console output.
- [x] `:calc formula <expr> [for <var>] -mb` renders a LaTeX-like formula block with target variable support.
- [x] `:formula <expr> [for <var>]` renders a LaTeX-like terminal formula without evaluating when requested.
- [x] `:plot <expr> [domain/options]` opens or prints a mathematical function plot through a typed ratatui Console block.
- [x] `:base <value> from <base> to <base|range>` converts between numeral systems from base 2 to base 16.
- [ ] `:units <expr>` is a later optional engineering unit-conversion command.
- [ ] `:stats <values/options>` is a later optional statistics helper.
- [ ] `:hash <algo> <text|file>` is a later optional utility command with explicit file permission handling.
- [ ] `:json <query/options>` is a later optional JSON formatter/query command.
- [ ] `:regex <pattern> <text/options>` is a later optional regex playground command.
- [ ] `:time <expr/options>` is a later optional time/date helper.
- [ ] `:bytes <expr>` is a later optional byte-size conversion helper.
- [ ] `:color <expr>` is a later optional hex/rgb/hsl color conversion helper.
- [ ] `:matrix <expr>` is a later optional matrix math helper after scalar calculator behavior is stable.
- [ ] `:encode <mode> <value>` is a later optional helper for base64, ASCII, UUID, random tokens, and related encodings.
- [ ] `:toy <name>` starts a visual terminal toy backed by `ConsoleSession`.
- [ ] `:games` lists available games.
- [ ] `:play <game>` starts a game-backed `ConsoleSession`.
- [ ] `:games --tags <tag>` filters games by type such as puzzle, arcade, word, realtime, or network.
- [ ] `:game pause`, `:game resume`, `:game restart`, and `:game quit` control the active game session.
- [ ] `:scores` and `:scores <game>` show local score summaries where a game supports scoring.
- [ ] Optional aliases such as `games`, `play`, and `scores` must be config-gated and documented as aliases for `:` commands.

## Mathematical Function Modulator / Plotter

- [x] Implement the "modulator" as a ratatui-style mathematical function plotter, not just a generic module switcher.
- [x] Support commands such as `:plot sin(x)`, `:plot x^2 from -10..10`, and `:plot exp(-x^2) --samples auto`.
- [x] Support domain syntax for x ranges, for example `from -10..10`, `x=-pi..pi`, or a clearly documented equivalent.
- [x] Support y-range syntax such as `y=-2..2` for manual vertical bounds.
- [x] Support sample count options with strict min/max limits.
- [x] Support render modes: line, points, bars, and compact sparkline where useful.
- [x] Add a bounded plot cache keyed by expression, variable, domain, samples, render mode, and canvas size.
- [x] Return typed plot data from the math extension instead of flattening production plots into strings.
- [x] Render plot command blocks with ratatui `Chart`/typed widgets in the Console UI.
- [x] Keep ASCII plot canvas as a width-safe fallback and test/debug representation.
- [x] Route non-plot visual math blocks, including the trigonometric unit circle, through typed ratatui renderers instead of styled text output.
- [x] Keep ASCII visuals only as fallback/history/debug output for terminals or areas too small for typed rendering.
- [x] Ensure all current and future graph-like visuals expose typed ratatui data first, with string output only as fallback/debug history.
- [x] Render explicit `x` and `y` axes/titles for all typed plot graphs and preserve the width-safe fallback.
- [x] Support pan and zoom through interactive plot sessions after the non-interactive renderer is stable.
- [ ] Support cursor inspection of approximate x/y values in interactive plot sessions.
- [ ] Support multiple functions on one plot after the single-function path is stable.
- [x] Support constants: `pi`, `e`, `tau`.
- [x] Support arithmetic operators, powers, unary operators, parentheses, and common functions.
- [x] Support trigonometric functions: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`.
- [x] Support hyperbolic functions: `sinh`, `cosh`, `tanh`.
- [x] Support logarithmic/exponential functions: `ln`, `log`, `log10`, `exp`.
- [x] Support numeric helpers: `sqrt`, `abs`, `floor`, `ceil`, `round`, `min`, `max`.
- [ ] Support piecewise expressions only after the core parser and renderer are verified.
- [x] Detect discontinuities and asymptotes without drawing misleading vertical walls.
- [x] Clip out-of-range values safely.
- [x] Represent NaN and infinite values explicitly in sampling logic, not as panics.
- [x] Compute y-range automatically with robust handling for outliers.
- [x] Allow manual y-range when automatic range is not useful.
- [x] Render axes and labels when space allows.
- [x] Degrade gracefully in very small terminal sizes.
- [x] Add unit tests for expression parsing, sampling, discontinuity handling, clipping, and range selection.
- [ ] Add wider snapshot coverage for narrow, medium, and wide terminal plot widgets.

## Engineering Calculator

- [x] Parse `for <var>` before relation parsing for all relation-style `:calc` inputs, not only explicit `solve`.
- [x] Allow symbolic parameter variables when the target variable is explicitly provided.
- [x] Solve symbolic linear/quadratic equations such as `a*x^2 + b*x + c = 0 for x` exactly.
- [x] Return exact trigonometric families for equations such as `sin(x) = 1` before numeric fallback.
- [x] Render a rich trigonometric unit circle in `-mb` for supported trig equations and inequalities, including `sin`/`cos` axes, exact solution points, inequality bounds, and highlighted solution arcs.
- [x] Render the trigonometric unit circle through a typed ratatui visual block in the Console UI, not only as styled text rows.
- [x] Recognize exact zero equations with positive trig powers such as `cos(x)^2 = 0` and `sin(x)^2 = 0` before numeric fallback.
- [x] Render exact pi-family solutions for supported trig-power zero equations in compact and Math Block output; numeric lists over broad domains must require `-num`/numeric fallback only when exact handling is impossible.
- [x] Render exact result expressions in the Math Block formula section, including roots and fractions.
- [x] Implement a real expression evaluator, not a string-based toy calculator.
- [x] Use a small local Pratt parser for this milestone to avoid adding calculator dependencies before the shared AST/API shape is proven.
- [x] Keep dependency evaluation open for later symbolic math, arbitrary precision, complex numbers, or CAS-level features if local numeric solving is no longer enough.
- [x] Support operator precedence and associativity correctly.
- [x] Support unary plus/minus and nested parentheses.
- [x] Support functions shared with the plotter.
- [x] Support constants shared with the plotter.
- [x] Support `ans` for the previous calculator result.
- [x] Support named variables with an explicit command such as `:calc let a = 42`.
- [x] Support one-variable numeric equation solving, for example `:calc solve x^2 - 4 = 0 for x`.
- [x] Support one-variable numeric inequality solving over bounded domains, for example `:calc solve sin(x) > 0 from -pi..pi`.
- [x] Keep systems of equations, differential equations, symbolic algebra, and calculus for later phases after the function modulator exists.
- [x] Support clear diagnostics with caret/location where possible.
- [x] Default `:calc` output should stay compact and exact-first.
- [x] Numeric approximation should require `-num`/`--num` or an explicit Math Block action.
- [x] Rich Math Block should require `-mb` and should not be the default command output.
- [x] Math Block output must include input, exact result, LaTeX-like rendering, ASCII fallback, numeric approximation, domain, and vars.
- [x] Math Block vars section must support multiple variables from the start so systems of equations can reuse the layout later.
- [ ] Support integer and floating-point paths where it matters for exactness.
- [ ] Consider rational/exact arithmetic as a later phase if the dependency and UX are justified.
- [ ] Consider complex numbers as a later phase after real-valued math is stable.
- [x] Prevent panics, uncontrolled NaN propagation, and silent overflow where Rust types can report errors.
- [ ] Add unit tests for precedence, functions, variables, constants, errors, and edge cases.
- [ ] Add integration tests through the Console command router.

## Base Conversion

- [x] Expand `:base` to support every base from 2 through 16.
- [x] Accept explicit source and target bases: `:base ff from 16 to 2`.
- [x] Accept target ranges or tables: `:base 255 to 2..16`.
- [x] Validate every digit against the source base.
- [x] Support uppercase and lowercase digits.
- [x] Support `_` separators in numeric input where unambiguous.
- [x] Support negative values.
- [x] Print clean table output when converting to several bases.
- [x] Print specific error messages for invalid base, invalid digit, empty value, and overflow.
- [x] Consider big integer support only if it does not add disproportionate complexity.
- [ ] Add unit tests for every base 2..16 and invalid digit paths.

## LaTeX-Like Formula Rendering

- [x] Add a terminal formula renderer driven by the parsed expression AST.
- [x] Treat LaTeX-like rendering as production UI: width-aware layout tree, deterministic fallback, no overflow, no broken borders, no glyph assumptions without fallback.
- [x] Support pretty powers where terminal width and font support make it readable.
- [x] Support subscripts/indices where the expression model needs them.
- [x] Support fractions with multi-line numerator/denominator when there is enough height.
- [x] Support roots, grouped terms, and function calls.
- [x] Provide an ASCII fallback for terminals where Unicode width or glyph support is unsafe.
- [x] Use width-aware layout logic, for example via `unicode-width`, before drawing into ratatui buffers.
- [x] Never let formula rendering overflow into borders or neighboring blocks.
- [x] Expose pretty output through `:formula <expr>` and rich `:calc ... -mb` output.
- [x] Expose rich formula/result layout through `:calc ... -mb` and keep compact `:calc` output plain.
- [x] Expose `:calc --pretty <expr>` as an alias for formula rendering through the same AST renderer.
- [ ] Add snapshot tests for powers, fractions, nested expressions, narrow width fallback, and wide width output.

## Mini-Games

- [ ] Implement games as Console extensions backed by `ConsoleSession`.
- [ ] Launch games inside an interactive Console output block, not a fullscreen tab.
- [ ] Represent active games as a `GameBlock` or equivalent Console block that owns render area, input, tick state, and lifecycle.
- [ ] Exit active games consistently with `Esc`, `q`, or `Ctrl+C`.
- [ ] On finish or quit, leave a compact history summary such as game id, status, score, and elapsed time.
- [ ] Keep game summaries searchable/readable in normal Console history.
- [ ] Start with one small game to prove the API before adding several.
- [ ] Candidate first games: Snake, 2048, Minesweeper, Tetris, Netwalk.
- [ ] Candidate later arcade games: Pong, Breakout.
- [ ] Candidate later word games: Wordle, Hangman, Typing trainer.
- [ ] Candidate later grid/puzzle games: Pipes, Sokoban, Conway Life.
- [ ] Candidate later adventure games: tiny Rogue-lite micro dungeon.
- [ ] Candidate later visual/toy sessions: Matrix rain, Mandelbrot explorer, rain, clock, starfield, plasma.
- [ ] Candidate project-specific game: Packet Runner with packet loss, firewall, NAT, and route-choice mechanics.
- [ ] Keep all games keyboard-only and terminal-native.
- [ ] Support arrow keys and WASD where appropriate.
- [ ] Support per-game options such as `:play tetris --level 3` and `:play mines --size 16x16 --mines 40`.
- [ ] Define per-game tags so `:games --tags puzzle` and similar filters are useful.
- [ ] Use deterministic seeded RNG in tests.
- [ ] Keep board memory fixed or tightly bounded.
- [ ] Keep game ticks capped and independent of render frequency.
- [ ] Support pause, resume, restart, and quit consistently.
- [ ] Ensure leaving a game returns the Console to normal shell input cleanly.
- [ ] First implementation should be Snake because it validates realtime tick, input, render, pause, quit, and summary flow.
- [ ] Second implementation should be 2048 because it validates turn-based input without realtime pressure.
- [ ] Add Minesweeper and Tetris after the runtime handles both realtime and turn-based models.
- [ ] Add Netwalk after the basics because it gives the project a unique network-themed game.
- [ ] Add unit tests for game state transitions.
- [ ] Add render/snapshot tests for compact and full-width game layouts.

## User Modules and Plugin System

- [ ] Phase 1: built-in modules only, behind the same extension API.
- [ ] Phase 2: manifest-driven content for built-in modules, such as Sokoban levels, Wordle dictionaries, Minesweeper presets, and game settings.
- [ ] Phase 3: user-authored script modules with a manifest and strict permissions.
- [ ] Phase 4: external process plugins using a stable JSON-lines protocol if script modules are not enough.
- [ ] Do not implement native dynamic Rust plugins with `.so`/`.dll` loading in the early phases because ABI stability, safety, and versioning are disproportionate risk.
- [ ] Evaluate a small scripting engine such as Rhai only after built-in APIs are stable.
- [ ] Define `ModuleManifest` fields: id, name, version, kind, description, commands, session types, tags, permissions, entrypoint.
- [ ] Allow manifests to describe command help text and examples.
- [ ] Default-deny filesystem access.
- [ ] Default-deny network access.
- [ ] Default-deny shell/process execution.
- [ ] Default-deny environment access except explicitly passed safe values.
- [ ] Show dangerous permissions in `:mod info` before a user enables a module.
- [ ] Add execution timeouts for user modules.
- [ ] Add kill/cancel handling for long-running external plugins.
- [ ] Add output-size limits for user modules.
- [ ] Add clear error reporting for permission denial and module crashes.
- [ ] Use platform-appropriate module directories: `%APPDATA%` on Windows and `$XDG_CONFIG_HOME` on Linux.
- [ ] Support project-local modules only after trust rules are explicit.
- [ ] Never auto-run project-local modules without user trust.
- [ ] External process plugins should use messages such as `init`, `key`, `tick`, and `resize`.
- [ ] External process plugins should return bounded frame lines plus status over JSON-lines.
- [ ] Add manifest validation tests and plugin protocol tests before enabling external modules.

## Console UI and Design

- [ ] Create a cohesive Console extension visual language before adding many modules.
- [ ] Use typed ratatui widgets as the default production path for every graph/visual module; do not add new graph-like features as plain text first.
- [x] Fix Console Tab ghost-completion so accepting a suggestion never inserts a literal tab/indent and never requires Backspace to refresh the visible suggestion.
- [ ] Keep the design terminal-native but modern, similar in quality to the Network tab.
- [ ] Keep games visually inside Console output history rather than visually acting like a separate tab.
- [ ] Prefer ASCII-safe boards for first implementations to avoid repeating mojibake issues.
- [ ] Use compact status bars for game title, score, speed/level, elapsed time, and key hints.
- [ ] Support theme presets such as classic, matrix, amber, and mono after the base styling is stable.
- [ ] Use consistent title, subtitle, border, focus, and error styles.
- [ ] Avoid nested decorative boxes unless the border is functionally meaningful.
- [ ] Keep output dense enough for terminal work but not cramped.
- [ ] Ensure every block works at narrow widths.
- [ ] Clamp or wrap titles so they never overwrite borders.
- [ ] Keep status text stable so values do not cause layout jitter.
- [ ] Use clear keyboard hints only where they are actionable.
- [ ] Keep color meaningful, not decorative noise.
- [ ] Ensure all non-ASCII glyphs have fallback or are proven width-safe.
- [ ] Add render tests for common terminal sizes.
- [x] Math Block must be visually heavier than normal output but still Console-native: one functional bordered block, stable title/status rows, and no nested decorative boxes.
- [x] Math Block must degrade cleanly at narrow widths by prioritizing result, fallback, and numeric value over secondary metadata.

## Network Graph Title/Border Bug

- [x] Reproduce the reported upload graph issue from the screenshot.
- [x] Inspect `render_sparkline_graph` title construction in `src/ui/tabs/network.rs`.
- [x] Confirm whether the dynamic title segment such as sample seconds is too long for the graph width.
- [x] Fix title layout so changing values never replace the right border.
- [x] Prefer moving volatile sample-count text out of the block title or width-clamping it.
- [x] Keep download/upload graph titles visually consistent.
- [x] Add a narrow-width render regression test for both graphs.
- [x] Add a full-width render regression test for both graphs.
- [x] Verify the right border remains intact while values change.

## Verification Plan

- [x] Run `cargo fmt --all` after implementation changes.
- [x] Run `cargo test` after each coherent module milestone.
- [x] Run `cargo check --all-targets` before marking a milestone complete.
- [x] Run Linux `cross check --target x86_64-unknown-linux-gnu --all-targets` after cross-platform changes.
- [x] Run Linux cross-test coverage after behavior changes; use split `--lib`, `--bins`, and `--doc` when monolithic `--all-targets` is too slow.
- [x] Run `git diff --check` before every review update.
- [ ] Add unit tests for command routing, calculator parsing/evaluation, base conversion, plot sampling, formula layout, game state, and plugin manifests.
- [ ] Add render/snapshot tests for Console extension blocks, plot widgets, formula output, games, and the Network graph bug.
- [ ] Add performance checks or targeted benchmarks for graph sampling, game ticks, and formula layout if they become non-trivial.
- [ ] Manually verify Linux Console keyboard flow after interactive modules are introduced.
- [ ] Manually verify Windows Console behavior is unchanged for normal shell commands.

## Staff-Engineer Review Gate

- [ ] Does the extension router preserve normal shell intent?
- [ ] Does every module have bounded CPU, memory, output, and tick behavior?
- [ ] Does every UI block stay inside its ratatui area at small sizes?
- [ ] Does the calculator produce correct results for non-trivial engineering expressions?
- [ ] Does `:base` work for every base from 2 through 16 with clear errors?
- [ ] Does formula rendering degrade cleanly when pretty output is not safe?
- [ ] Are user modules disabled or sandboxed by default?
- [ ] Are Network graph borders stable under changing values?
- [ ] Are tests strong enough that regressions would be caught before release?
- [ ] Is the implementation smaller and clearer than a direct hardcoded feature pile would be?

## Review Notes

- The mathematical modulator means a function plotter/renderer, not only a generic module manager.
- Games are intentionally later than the extension API, calculator, plotter, formula renderer, and Network graph bug.
- Plugin support is intentionally phased after built-ins so the API can be proven without security risk.
- The current plan favors explicit internal commands to avoid breaking Linux shell behavior that was just repaired.
- Network graph title rendering now removes the volatile sample-count title segment and width-clamps graph titles before ratatui draws the border.
- Console Extension Platform foundation now routes only `:`-prefixed commands to `ConsoleCommandRouter`; non-prefixed input remains normal shell input.
- The initial built-in Console Core extension provides `:help` and `:mods`; future math/games/toys modules should register through the same registry.
- `cross test --target x86_64-unknown-linux-gnu --all-targets` timed out as a monolithic command once, then the equivalent Linux test surface passed when split into `--lib`, `--bins`, and `--doc`.
- Final milestone verification passed: `cargo fmt --all`, `cargo test`, `cargo check --all-targets`, `cross check --target x86_64-unknown-linux-gnu --all-targets`, split Linux `cross test` for `--lib`/`--bins`/`--doc`, and `git diff --check`.
- Math module implementation plan: add a shared `app::math` core with AST/parser/evaluator/base conversion/numeric solver, then expose it through a thin built-in Console extension for `:base` and `:calc`.
- Math milestone verification is intentionally static-only per current request; do not run `cargo test` or full compilation in this step.
- Math module implementation completed with a shared local Pratt parser/AST/evaluator, i128-backed base conversion, one-variable numeric equation solving, and bounded-domain inequality solving.
- Static-only verification passed for this milestone: `cargo check --all-targets` and `cross check --target x86_64-unknown-linux-gnu --all-targets`.
- Solver correction: strict inequality interval brackets now treat internal roots as open boundaries instead of trusting floating-point sign at an approximate root.
- Solver correction: equality no-root output now explicitly says the numeric solver only searched the displayed bounded domain.
- Math Block design correction: rich block rendering is opt-in via `-mb`; normal `:calc` remains compact exact-first output, and `-num`/`--num` controls approximation.
- Formula target requirement: complex formulas with multiple variables must accept `for <var>`, for example `:calc formula "(-b + sqrt(b^2 - 4*a*c)) / (2*a)" for x -mb`.
- Exact-first output implemented for compact `:calc`; `-num` appends approximations and `-mb` renders a bounded Math Block with input, exact result, formula, fallback, approx, domain, and vars.
- Formula renderer now uses a measured AST layout tree for fractions, powers, square roots, grouped terms, and function calls, then falls back to ASCII when width is insufficient.
- Exact solver now handles linear/quadratic polynomial equations, basic radical equations such as `sqrt(x)=7` and `sqrt(x)/8=49`, and basic sine/cosine zero-comparison families with pi-form intervals.
- Linux `cross check` hit Docker BuildKit `lease does not exist` once, then passed on rerun; Rust static checks passed.
- Correction pass fixed relation-level `for <var>` parsing, so `:calc a*x^2 + b*x + c = 0 for x` no longer treats `for` as an unknown symbol.
- Explicit solve targets now allow unassigned non-target symbols as exact-solver parameters before numeric fallback is considered.
- Symbolic linear/quadratic equations now produce exact parameterized roots, including the quadratic formula for `a*x^2 + b*x + c = 0 for x`.
- Trigonometric point equations such as `sin(x) = 1`, `sin(x) = -1`, `cos(x) = 1`, and `cos(x) = -1` now return exact pi-family solutions before numeric fallback.
- Math Block formula sections now render exact result expressions, so roots/fractions are shown from the solution rather than repeating the input equation.
- Math Block top borders now compute width exactly and no longer self-clamp into `...+`.
- Correction pass static verification passed: `cargo check --all-targets`, Linux `cross check --target x86_64-unknown-linux-gnu --all-targets`, and `git diff --check`; existing warnings remain outside this math module work.
- Formula renderer readiness check: `:formula <expr>`, AST-driven powers/fractions/roots, and ASCII/width-safe fallback were already present; this milestone added `:calc --pretty <expr>` and identifier-index layout for names such as `x_1`.
- Plotter milestone added `:plot` with bounded sampling, explicit `from a..b` and `x=a..b` domains, manual `y=a..b`, line/points/bars/sparkline modes, axes/labels, robust auto y-range, clipping, invalid-value handling, and discontinuity breaks.
- Plot cache is bounded and keyed by expression, target variable, domain, y-range, sample count, mode, canvas size, and assigned parameter values.
- Interactive plot zoom/pan remains blocked by the existing ConsoleSession integration gap: `StartSession` is defined but not yet wired into Console event/render handling, so this milestone does not expose fake interactive controls.
- Plotter milestone static verification passed: `cargo check --all-targets`, Linux `cross check --target x86_64-unknown-linux-gnu --all-targets`, `git diff --check`, and a direct trailing-whitespace scan for the new `src/app/math/plot.rs` file.
- Plotter UI correction: `:plot` now returns typed Console plot output with sampled series data; the Console tab renders it through ratatui `Chart`/`Sparkline` widgets and keeps the ASCII canvas only as fallback/debug output.
- Plotter UI tests cover typed command block storage, ratatui chart rendering, narrow fallback rendering, and discontinuity-aware plot series splitting.
- Plotter UI verification passed: `cargo fmt --all`, `cargo test` with 102 tests, `cargo check --all-targets`, Linux `cross check --target x86_64-unknown-linux-gnu --all-targets`, and `git diff --check`.
- Sixth milestone added styled Console output spans, so Math Block can render colored ASCII-safe visuals without ANSI escape leakage.
- Trig Math Block visuals now draw a one-period `sin`/`cos` unit circle for supported `sin(x)`/`cos(x)` equations and inequalities, with exact points, inequality boundaries, and highlighted solution arcs.
- `ConsoleSession` is now wired into command blocks, key dispatch, capped ticking, typed rendering, quit/finish summaries, Ctrl+C handling, and history recording.
- `:plot <expr> -i` starts an interactive plot session with arrow/HJKL pan, `+`/`-` zoom, `r`/`0` reset, and `q`/`Esc`/`Ctrl+C` quit; cursor inspect remains intentionally deferred.
- Typed plot blocks now expose explicit `x`/`y` axis titles and visible x/y range labels; ASCII fallback labels the y axis as well.
- Sixth milestone verification passed: `cargo fmt --all`, `cargo test` with 107 tests, `cargo check --all-targets`, Linux `cross check --target x86_64-unknown-linux-gnu --all-targets`, and `git diff --check`.
- Seventh milestone added `CommandOutput::Visual` and a typed `ConsoleVisualBlock` path so trigonometric unit-circle visuals render through ratatui canvas, while ASCII lines remain fallback/history output.
- Console Tab completion now accepts both `KeyCode::Tab` and terminal-emitted `KeyCode::Char('\t')` without inserting a literal tab or requiring Backspace to refresh the suggestion.
- Exact trig solving now handles zero equations with positive trig powers such as `cos(x)^2 = 0` as pi-family results before numeric fallback; powered trig inequalities are deliberately not reduced to an incorrect base-sign inequality.
- Seventh milestone verification passed: `cargo fmt --all`, `cargo test` with 113 tests, `cargo check --all-targets`, Linux `cross check --target x86_64-unknown-linux-gnu --all-targets`, and `git diff --check`.

## Linux Disk Interaction And Hot-Plug

- [x] Make the Filesystems panel keyboard-focusable from the Disk tab.
- [x] Add volume selection with Up/Down and panel focus switching with Left/Right.
- [x] Expand or collapse grouped mount points with Enter or Space.
- [x] Preserve expansion state across monitor refreshes with stable filesystem keys.
- [x] Accept desktop automounts below `/run/media` without exposing unrelated runtime mounts.
- [x] Detect removable/hot-plug block devices from `lsblk`.
- [x] Show removable USB storage as `Removable USB`.
- [x] Add regression coverage for `/run/media` filesystems and expanded mount rendering.
- [x] Verify the connected SanDisk exFAT filesystem appears in both Physical Disks and Filesystems.
- [x] Verify focus switching and mount expansion in a real PTY session.

Disk interaction verification passed: `cargo fmt --all`, `cargo test --lib` with 141 tests, `cargo test --all-targets`, `cargo build --bin TUI`, `cargo check --all-targets`, targeted thread-safety Clippy lints, and `git diff --check`.
