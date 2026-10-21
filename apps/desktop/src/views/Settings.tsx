import { statusDetail } from '../status'
import { StatusBadge } from '../components/StatusBadge'
import type { BackendStatus, HostInfo } from '../types'

type Props = {
  backends: BackendStatus[] | null
  host: HostInfo | null
  appearance: string
  onRefresh: () => void
}

export function Settings({ backends, host, appearance, onRefresh }: Props) {
  return (
    <div className="settings-page">
      <section>
        <div className="section-head">
          <div>
            <h2>运行软件</h2>
            <p>这台电脑上用来承载 Agent 环境的软件。装好其中一个即可。</p>
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
            <h2>环境系统</h2>
            <p>每个环境里安装的系统是 mixumOS，这里是它的设置。</p>
          </div>
        </div>
        <div className="settings-list">
          <div className="setting-row">
            <div>
              <strong>快照与回退</strong>
              <span className="muted">自动保存工作进度，可以随时回到之前的状态</span>
            </div>
            <span className="mono muted">即将开放</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>内存快照</strong>
              <span className="muted">保留正在运行的进程；目前要求来源和目标使用同一内核</span>
            </div>
            <span className="mono muted">同一内核</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>系统镜像</strong>
              <span className="muted">环境使用的 mixumOS 版本与更新方式</span>
            </div>
            <span className="mono muted">即将开放</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>磁盘占用</strong>
              <span className="muted">快照与镜像占用的空间，可清理未使用的部分</span>
            </div>
            <span className="mono muted">即将开放</span>
          </div>
        </div>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>应用设置</h2>
            <p>MART 自己的设置。</p>
          </div>
        </div>
        <div className="settings-list">
          <div className="setting-row">
            <div>
              <strong>外观</strong>
              <span className="muted">中性黑白灰，跟随系统，也可在标题栏手动切换</span>
            </div>
            <span className="mono muted">{appearance}</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>连接安全</strong>
              <span className="muted">首次连接环境时记录证书指纹，之后自动核对</span>
            </div>
            <span className="mono muted">已启用</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>匿名统计</strong>
              <span className="muted">默认关闭；开启后只上报数量类信息，不包含任何内容</span>
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
