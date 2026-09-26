# Security policy

## Reporting a vulnerability

Please **don't** open a public issue for security problems. Report them
privately through GitHub's
[private vulnerability reporting](https://github.com/andrew11155/FileFlier/security/advisories/new)
(Security → Report a vulnerability).

Include:
- the File Flier version, and whether you use the Flatpak or the native build
- steps to reproduce
- what happens, and what you expected

You should get an answer within a week.

## Scope

In scope: anything that could make File Flier lose, overwrite or expose data
unexpectedly, run code without the user choosing to, or escape its Flatpak
permissions.

Out of scope: the Flatpak's broad filesystem access (`--filesystem=host`). A
file manager needs it, and it is documented in the README.
