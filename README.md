# 🤹 Playpen

Your LLM 👶 behaves like a clown 🤡 and eats every bait 🎣 it can find?
Put your agent in a playpen and hope that it won't escape 🎰

## Notes

https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html
https://www.man7.org/linux/man-pages/man2/seccomp.2.html
https://www.man7.org/linux/man-pages/man2/PR_SET_SECCOMP.2const.html
https://man7.org/linux/man-pages/man3/seccomp_init.3.html

-> use seccomp with ebpf filters SECCOMP_SET_MODE_FILTER to pass syscalls for handling to userspace via SECCOMP_RET_USER_NOTIF


https://docs.kernel.org/userspace-api/landlock.html#
https://landlock.io/news/

Use landlock to limit file access? It is a very convenient API, but we have to check syscalls before landlock handles them. Landlock doesnt give us a hookpoint so we cannot delay the decision. Maybe we can still use it to limit socket access?


