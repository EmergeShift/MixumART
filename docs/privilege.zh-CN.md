# 权限模型

普通任务以 `mixum` 用户运行。`mixumd` 只在 cgroups、快照/CRIU 和显示初始化时以 root
身份充当 supervisor。数据目录仅 root 可访问；`work` 和 `home` 属于 `mixum`。

## sudo

Agent 执行 `sudo <program> <args>`；`/usr/local/bin/sudo` 是一个非 setuid 客户端，
把原始 argv 提交到 `/run/mixum/sudo.sock`。broker 会：

1. 向已连接的 Admin 会话推送 `privilege.request`；
2. 等待 `privilege.decision`（120 秒）；
3. 用清理过的环境以 root 执行一次原始 argv；
4. 返回 stdout/stderr/exit，并写审计记录。

没有批准就不会执行。不存在 root shell。

批准路径：

- MixumART 对话框：显示命令、风险、任务和 agent 的理由；批准前需要宿主密码或
  操作系统生物识别提示。
- CLI 回退：`mixum privilege watch <env>`，然后
  `mixum privilege approve <env> <id>`（先在宿主执行 `sudo -v`）。

## Root 容器

```bash
mixum env create --title risk --image mixumos:0.1.5 --root
```

CLI 要求交互式终端，并逐字确认 `ROOT`；MCP 工具不暴露 `--root`。root 容器设置
`MIXUM_TASK_USER=root`，因此容器内的任务以 root 运行。这不是宿主 root，但可以修改
容器系统、读取已挂载的密钥、关闭审计/agentd，并扩大影响范围。优先使用默认
`mixum` 用户 + 逐命令批准。
