import { statusLabel, statusTone } from '../status'
import type { RuntimeStatus } from '../types'

export function StatusBadge({ status }: { status: RuntimeStatus }) {
  return (
    <span className={`badge ${statusTone(status)}`}>
      <span className="dot" />
      {statusLabel(status)}
    </span>
  )
}
