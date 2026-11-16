# Security policy

Only the current `main` branch is supported for security fixes. Older commits
and upstream rbxport releases are not maintained by this fork.

## Report privately

Use [GitHub private vulnerability reporting](https://github.com/The-Greg-O/wreckordbox/security/advisories/new)
(the repository’s Security tab → Report a vulnerability). Do not open a public
issue containing exploit details or private data. If that button is unavailable,
private reporting has not yet been enabled: open an issue asking the maintainer
to enable it, without disclosing the vulnerability, and wait to submit details.

Include the affected commit, operating system, impact, and reproduction steps
using a disposable fixture. Do not send real music, Library databases, Stick
contents, credentials, or personal paths. This is a single-maintainer project;
there is no guaranteed response time.

## Scope

The desktop app, Library service, `wreckordbox` CLI and `wreckordbox-mcp` server
are in scope, including flaws that expose or corrupt a user’s Library or Sticks,
bypass Library or Stick Protection, or allow unauthorized code execution or
file access. Test only against disposable Libraries and temporary directories,
never another person’s data or a mounted performance Stick. Ordinary feature
requests and non-security bugs belong in GitHub Issues.
