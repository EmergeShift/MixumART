import { statusDetail } from '../status'
import { StatusBadge } from '../components/StatusBadge'
import type { BackendStatus, HostInfo } from '../types'

type Props = {
  backends: BackendStatus[] | null
  host: HostInfo | null
  onRefresh: () => void
}

export function Settings({ backends, host, onRefresh }: Props) {
  return (
    <div className="settings-page">
      <section>
        <div className="section-head">
          <div>
            <h2>运行时</h2>
            <p className="muted">宿主机上可用于承载 mixumOS 的容器运行时。</p>
          </div>
          <button className="ghost" onClick={onRefresh}>
            重新检测
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

      <section>
        <div className="section-head">
          <div>
            <h2>mixumOS</h2>
            <p className="muted">容器内系统基座的设置。由节点的 sys.info 与配置接口提供。</p>
          </div>
        </div>
        <div className="settings-list">
          <div className="setting-row">
            <div>
              <strong>快照后端</strong>
              <span className="muted">disk / hybrid 快照的存储与物化后端</span>
            </div>
            <span className="mono muted">待接入</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>快照存储与 GC</strong>
              <span className="muted">CAS 容量、清理策略与保护窗口</span>
            </div>
            <span className="mono muted">待接入</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>内存快照</strong>
              <span className="muted">
                CRIU 仅在同内核节点可用；跨内核未验证（criu-feasibility.md）
              </span>
            </div>
            <span className="mono muted">同内核</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>镜像与更新</strong>
              <span className="muted">mixumos 镜像版本、更新通道与 pin</span>
            </div>
            <span className="mono muted">待接入</span>
          </div>
        </div>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>MART</h2>
            <p className="muted">宿主侧应用设置。</p>
          </div>
        </div>
        <div className="settings-list">
          <div className="setting-row">
            <div>
              <strong>外观</strong>
              <span className="muted">深色 Operator（当前唯一主题）</span>
            </div>
            <span className="mono muted">深色</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>节点证书</strong>
              <span className="muted">TOFU 指纹固定；首次连接需人工确认</span>
            </div>
            <span className="mono muted">TOFU</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>遥测</strong>
              <span className="muted">默认关闭；启用后仅聚合匿名指标（见 TRUST.md）</span>
            </div>
            <span className="mono muted">关闭</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>版本</strong>
              <span className="muted">{host ? `${host.os} · ${host.arch}` : '—'}</span>
            </div>
            <span className="mono">{host ? `v${host.appVersion}` : '—'}</span>
          </div>
        </div>
      </section>
    </div>
  )
}
