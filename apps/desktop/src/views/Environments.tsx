import { statusDetail } from '../status'
import type { BackendStatus } from '../types'
import { StatusBadge } from '../components/StatusBadge'

type Props = {
  backends: BackendStatus[] | null
  refreshing: boolean
  onRefresh: () => void
}

export function Environments({ backends, refreshing, onRefresh }: Props) {
  return (
    <section>
      <div className="section-head">
        <div>
          <h2>运行时</h2>
          <p className="muted">
            宿主机上可用于承载 mixumOS 的容器运行时。当前为只读探测，创建与启停随后续里程碑接入。
          </p>
        </div>
        <button className="ghost" onClick={onRefresh} disabled={refreshing}>
          {refreshing ? '检测中…' : '重新检测'}
        </button>
      </div>

      {backends === null ? (
        <p className="muted">正在检测…</p>
      ) : (
        <div className="cards">
          {backends.map((backend) => (
            <article className="card" key={backend.kind}>
              <div className="card-head">
                <h3>{backend.name}</h3>
                <StatusBadge status={backend.status} />
              </div>
              <p className="card-note">{statusDetail(backend.status)}</p>
              <code className="mono">{backend.kind}</code>
            </article>
          ))}
        </div>
      )}
    </section>
  )
}
