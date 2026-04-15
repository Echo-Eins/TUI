# Lessons

- When a user corrects missing project instructions, re-check untracked files as well as tracked files before concluding that a file is absent.
- Cross builds that install extra dev packages can hide native Linux link dependencies. Before claiming Linux-native build readiness, verify the default Linux dependency graph does not require undocumented link-time system libraries.
