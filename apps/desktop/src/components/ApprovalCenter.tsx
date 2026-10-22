import { useState } from 'react'

import {
  approveRootApproval,
  denyApproval,
  errorText,
  type ApprovalView,
} from '../api'

export function ApprovalCenter({
  approvals,
  onClose,
  onChanged,
}: {
  approvals: ApprovalView[]
  onClose: () => void
  onChanged: () => void
}) {
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function act(id: string, approve: boolean) {
    setBusy(id)
    setError(null)
    try {
      if (approve) {
        await approveRootApproval(id)
      } else {
        await denyApproval(id)
      }
      onChanged()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <div className="privilege-dialog approval-center" role="dialog" aria-modal="true" aria-label="Agent 审批">
        <div className="privilege-head">
          <div>
            <h2>Agent 审批</h2>
            <p className="privilege-note">root 容器与 Chrome 域名请求。浏览器请求批准后仍需在终端完成 share。</p>
          </div>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭">×</button>
        </div>
        {approvals.length === 0 && <p className="privilege-note">没有待处理请求。</p>}
        {approvals.map((item) => (
          <div className="approval-card" key={item.id}>
            <div className="approval-card-head">
              <span className={`risk risk-${item.kind === 'root' ? 'destructive' : 'network'}`}>
                {item.kind === 'root' ? 'ROOT' : 'BROWSER'}
              </span>
              <strong>{item.title}</strong>
              {item.approved && <span className="pill">已批准，待创建</span>}
            </div>
            {item.domains.length > 0 && (
              <p className="privilege-note">域名：{item.domains.join(', ')}</p>
            )}
            {item.agentStance && (
              <p className="privilege-note">
                Agent 态度：{item.agentStance}
                {item.agentReason ? ` — ${item.agentReason}` : ''}
              </p>
            )}
            {item.kind === 'browser' ? (
              <pre className="privilege-command">{item.action}</pre>
            ) : (
              <div className="privilege-actions">
                <button type="button" className="btn ghost" disabled={busy === item.id} onClick={() => void act(item.id, false)}>
                  拒绝
                </button>
                <button type="button" className="btn primary" disabled={busy === item.id || item.approved} onClick={() => void act(item.id, true)}>
                  {item.approved ? '已批准' : busy === item.id ? '认证中…' : '批准 root'}
                </button>
              </div>
            )}
          </div>
        ))}
        {error && <p className="form-error">{error}</p>}
      </div>
    </div>
  )
}
