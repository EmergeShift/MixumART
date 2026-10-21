import { useEffect, useMemo, useState } from 'react'

import { errorText, listContainers } from '../api'
import type { AgentMode, BackendStatus, Container, RuntimeKind } from '../types'

type Props = {
  backends: BackendStatus[] | null
}

const MODE_LABEL: Record<AgentMode, string> = {
  'as-tool': '直接使用',
  'sub-agent': '派发任务',
  isolated: '完全隔离',
}

const STATE_LABEL: Record<string, string> = {
  running: '正在运行',
  stopped: '已停止',
  paused: '已暂停',
  created: '未启动',
  unknown: '未知',
}

export function Containers({ backends }: Props) {
  const ready = useMemo(
    () => (backends ?? []).filter((backend) => backend.status.state === 'ready'),
    [backends],
  )
  const [kind, setKind] = useState<RuntimeKind | null>(null)
  const [containers, setContainers] = useState<Container[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    if (kind === null && ready.length > 0) setKind(ready[0].kind)
  }, [kind, ready])

  const load = async () => {
    if (kind === null) return
    setLoading(true)
    setError(null)
    try {
      setContainers(await listContainers(kind))
    } catch (err) {
      setError(errorText(err))
      setContainers([])
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind])

  return (
    <section>
      <div className="section-head">
        <div>
          <h2>全部环境</h2>
          <p>每个环境属于一个使用者。暂停、继续与终端入口会随环境管理功能上线。</p>
        </div>
        <button className="ghost" onClick={() => void load()} disabled={loading || kind === null}>
          {loading ? '读取中…' : '刷新'}
        </button>
      </div>

      {backends === null ? (
        <p className="muted">正在检测…</p>
      ) : ready.length === 0 ? (
        <div className="empty-panel">
          <h3>没有可用的运行软件</h3>
          <p>在「设置」页查看检测结果与安装提示。</p>
        </div>
      ) : (
        <>
          <div className="chips">
            {ready.map((backend) => (
              <button
                key={backend.kind}
                className={kind === backend.kind ? 'chip active' : 'chip'}
                onClick={() => setKind(backend.kind)}
              >
                {backend.name}
              </button>
            ))}
          </div>

          {error && <div className="error">{error}</div>}

          {containers === null ? (
            <p className="muted">读取中…</p>
          ) : containers.length === 0 ? (
            <div className="empty-panel">
              <h3>这里还没有环境</h3>
              <p>创建入口会随环境管理功能上线。</p>
            </div>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>状态</th>
                  <th>环境</th>
                  <th>谁在用</th>
                  <th>使用方式</th>
                  <th>系统镜像</th>
                  <th>端口</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {containers.map((container) => (
                  <tr key={container.id}>
                    <td>
                      <span className={`state state-${container.state}`}>
                        <span className="dot" />
                        {STATE_LABEL[container.state] ?? container.state}
                      </span>
                    </td>
                    <td>
                      <div className="cell-stack">
                        <span>{container.name}</span>
                        <span className="mono muted">{container.id.slice(0, 12)}</span>
                      </div>
                    </td>
                    <td>{container.agent?.name ?? <span className="muted">未标注</span>}</td>
                    <td>
                      {container.agent ? (
                        <span className="tag">{MODE_LABEL[container.agent.mode]}</span>
                      ) : (
                        <span className="muted">—</span>
                      )}
                    </td>
                    <td className="mono">{container.image}</td>
                    <td className="mono">
                      {container.ports.length === 0
                        ? '—'
                        : container.ports.map((port) => `${port.host}→${port.container}`).join(', ')}
                    </td>
                    <td className="row-actions">
                      <button className="ghost" disabled title="画面接入后可用">
                        监看
                      </button>
                      <button className="ghost" disabled title="终端接入后可用">
                        终端
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </section>
  )
}
