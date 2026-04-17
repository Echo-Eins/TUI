# Lessons

- When a user corrects missing project instructions, re-check untracked files as well as tracked files before concluding that a file is absent.
- Cross builds that install extra dev packages can hide native Linux link dependencies. Before claiming Linux-native build readiness, verify the default Linux dependency graph does not require undocumented link-time system libraries.
- When a user uses an ambiguous feature term, confirm the intended domain before expanding architecture around it; "modulator" here means a ratatui mathematical function plotter, not only a generic module system.
- Console secondary features must stay lazy and bounded by default so games, math tools, and plugins do not consume CPU or memory when the user is just using the shell.
