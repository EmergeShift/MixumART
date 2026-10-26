import { useState } from 'react'

import { decidePrivilege, decideRemotePrivilege, errorText } from '../api'

export type PrivilegeRequest = {
  id: string
  taskId?: string
  program: string
  args?: string[]
  cwd?: string
  reason?: string
  risk?: string
}

export type RemotePrivilegeTarget = {
  approvalId: string
  addr: string
  node?: string | null
  fingerprint?: string | null
}

export type PrivilegeEvent = {
  containerId: string
  request: PrivilegeRequest
  remote?: RemotePrivilegeTarget
}

export function PrivilegeDialog({
  event,
  onClose,
}: {
  event: PrivilegeEvent
  onClose: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const request = event.request
  const command = [request.program, ...(request.args ?? [])].join(' ')
  const risk = request.risk ?? 'unknown'

  async function decide(approve: boolean) {
    setBusy(true)
    setError(null)
    try {
      const decided = event.remote
        ? decideRemotePrivilege(event.remote.approvalId, approve, '')
        : decidePrivilege(event.containerId, request.id, approve, '')
      await decided
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <div className="privilege-dialog" role="dialog" aria-modal="true" aria-label="Agent 请求 root 权限">
        <div className="privilege-head">
          <div>
            <span className={`risk risk-${risk}`}>{risk}</span>
            <h2>Agent 请求执行 root 命令</h2>
          </div>
          <button type="button" className="icon-btn" onClick={onClose} disabled={busy} aria-label="关闭">
            ×
          </button>
        </div>
        <p className="privilege-note">
          {event.remote
            ? '只有这一条命令会在远程节点以 root 执行；凭据不会随事件发送，批准前请确认目标节点可信。'
            : '只有这一条命令会以 root 执行；不会给 Agent 一个 root shell。批准前请按系统提示完成密码/生物识别。'}
        </p>
        <pre className="privilege-command">{command}</pre>
        <dl className="privilege-meta">
          {event.remote && (
            <div>
              <dt>目标</dt>
              <dd>远程节点 {event.remote.node || '-'} · {event.remote.addr}</dd>
            </div>
          )}
          <div><dt>工作目录</dt><dd>{request.cwd || '-'}</dd></div>
          <div><dt>任务</dt><dd>{request.taskId || '-'}</dd></div>
          <div><dt>Agent 理由</dt><dd>{request.reason || '-'}</dd></div>
        </dl>
        {error && <p className="form-error">{error}</p>}
        <div className="privilege-actions">
          <button type="button" className="btn ghost" onClick={() => void decide(false)} disabled={busy}>
            拒绝
          </button>
          <button type="button" className="btn primary" onClick={() => void decide(true)} disabled={busy}>
            {busy ? (event.remote ? '提交审批…' : '等待系统认证…') : '批准一次'}
          </button>
        </div>
      </div>
    </div>
  )
}
