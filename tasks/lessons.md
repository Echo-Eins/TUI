# Lessons

- When a user corrects missing project instructions, re-check untracked files as well as tracked files before concluding that a file is absent.
- Cross builds that install extra dev packages can hide native Linux link dependencies. Before claiming Linux-native build readiness, verify the default Linux dependency graph does not require undocumented link-time system libraries.
- When a user uses an ambiguous feature term, confirm the intended domain before expanding architecture around it; "modulator" here means a ratatui mathematical function plotter, not only a generic module system.
- Console secondary features must stay lazy and bounded by default so games, math tools, and plugins do not consume CPU or memory when the user is just using the shell.
- Numeric solvers must treat internal relation roots as mathematical boundaries, not re-evaluate slightly shifted floating-point roots to decide strict inequality brackets.
- Math output should be exact-first: symbolic roots, pi-based trigonometric forms, and structured formulas are primary; decimal approximations require an explicit user action such as `--num`.
- Rich Math Block rendering must be opt-in with `-mb`; normal `:calc` output stays compact, while `-mb` shows input, exact LaTeX-like result, fallback, approximation, domain, and variables together.
- Formula commands for multi-variable expressions must accept `for <var>` so the rendered result can show the requested target variable, e.g. `formula <expr> for x -mb`.
- Relation-style calculator inputs must parse trailing query options such as `for <var>` before expression/relation parsing; otherwise option words become fake variables and break symbolic solving.
- Once the user explicitly selects a solve target, other unassigned symbols are parameters, not additional unknowns to reject before the exact symbolic solver gets a chance to run.
- When the user explicitly asks for ratatui visual modules, treat string/ASCII output as a fallback or smoke-test layer only; the production path must render through typed ratatui widgets or an interactive session render path.
- Ratatui is the priority renderer for all current and future Console graphs/visual blocks; ASCII/text drawings are acceptable only as fallback/history/debug output, not as the main UI path.
