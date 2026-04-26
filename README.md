# 🤹 Playpen

Your LLM 👶 behaves like a clown 🤡 and eats every bait 🎣 it can find?
Put your agent in a playpen and hope that it won't escape 🎰

> Your unhardened LLM asset exhibits critical behavioral vulnerabilities and is susceptible to all forms of adversarial exploitation.
> Isolate your agent within a sandboxed environment to mitigate unauthorized lateral movement and egress.
> Playpen delivers enterprise-level, defense-grade security hardening and counter-exploitation protocols.
> --- An anonymous LLM

A playpen is not a jail. It provides safety and security, but not fault isolation. Nor does playpen minimize attack surface or harden security. Instead playpen aims at maximizing visibility and cross-compartment interactions while securing the system, e.g., against attacks from a sporadically supervised LLM.


## Technical Idea

|                     | Read/write tool | Bash tool | Network APIs | Sudo commands |
|---------------------|-----------------|-----------|--------------|---------------|
| LLM self-review     | fuzzy           | fuzzy     | fuzzy        | fuzzy         |
| Tool-use policies   | effective       | fuzzy     | fuzzy        | fuzzy         |
| MCP policies        | effective       | fuzzy     | effective    | fuzzy         |
| Unix users          | effective       | effective | ineffective  | ineffective   |
| Filesystem policies | effective       | effective | inapplicable | incomplete    |
| Syscall policies    | inapplicable    | effective | inapplicable | effective     |

1. Traditional safeguards fail at policing bash and sudo commands.
2. Protect the filesystem by staging changes in an overlayfs.
3. Allow sudo commands while protect the OS integrity with syscall (seccomp) filters.


## Getting Started

Needs `sudo`, `bwrap`, `setpriv`, and `bash` in path.

## Notes

https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html
https://www.man7.org/linux/man-pages/man2/seccomp.2.html
https://www.man7.org/linux/man-pages/man2/PR_SET_SECCOMP.2const.html
https://man7.org/linux/man-pages/man3/seccomp_init.3.html

-> use seccomp with ebpf filters SECCOMP_SET_MODE_FILTER to pass syscalls for handling to userspace via SECCOMP_RET_USER_NOTIF


https://docs.kernel.org/userspace-api/landlock.html#
https://landlock.io/news/

Use landlock to limit file access? It is a very convenient API, but we have to check syscalls before landlock handles them. Landlock doesnt give us a hookpoint so we cannot delay the decision. Maybe we can still use it to limit socket access?


## TODO

...
