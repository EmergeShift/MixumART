# Privilege model

Normal tasks run as the `mixum` user. `mixumd` remains root only as a supervisor
for cgroups, snapshots/CRIU and display setup. The data directory is root-only;
`work` and `home` belong to `mixum`.

## sudo

Agent runs `sudo <program> <args>`; `/usr/local/bin/sudo` is a non-setuid client
that submits the exact argv to `/run/mixum/sudo.sock`. The broker:

1. pushes `privilege.request` to connected Admin sessions;
2. waits (120 s) for `privilege.decision`;
3. executes the exact argv as root, once, with a sanitized environment;
4. returns stdout/stderr/exit and writes an audit record.

No approval means no execution. There is no root shell.

Approval paths:

- MixumART dialog: shows command, risk, task and the agent's reason; host password
  or biometric OS prompt before approving.
- CLI fallback: `mixum privilege watch <env>` then
  `mixum privilege approve <env> <id>` (runs `sudo -v` on the host first).

## Root containers

```bash
mixum env create --title risk --image mixumos:0.1.5 --root
```

The CLI requires an interactive terminal and the literal confirmation `ROOT`; the
MCP tool does not expose `--root`. A root container sets `MIXUM_TASK_USER=root`,
so tasks are root inside that container. This is not host root, but it can modify
the container system, read mounted secrets, disable audit/agentd, and widen the
blast radius. Prefer the default `mixum` user plus per-command approval.
