import { useEffect, useMemo, useState } from 'react'

import { errorText, listContainers } from '../api'
import type { AgentMode, BackendStatus, Container, RuntimeKind } from '../types'

type Props = {
  backends: BackendStatus[] | null
}

const MODE_LABEL: Record<AgentMode, string> = {
  'as-tool': '作为工具',
  'sub-agent': 'Sub-agent',
  isolated: '隔离环境',
}

const STATE_LABEL: Record<string, string> = {
  running: '运行中',
  stopped: '已停止',
  paused: '已暂停',
  created: '已创建',
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
          <h2>环境列表</h2>
          <p className="muted">
            每个容器属于一个发起方；启停、终端、日志在运行时接口接入后从行内直接操作。
          </p>
        </div>
        <button className="ghost" onClick={() => void load()} disabled={loading || kind === null}>
          {loading ? '读取中…' : '刷新'}
        </button>
      </div>

      {backends === null ? (
        <p className="muted">正在检测运行时…</p>
      ) : ready.length === 0 ? (
        <div className="empty-panel">
          <h3>没有可用的运行时</h3>
          <p className="muted">在「设置」页查看检测结果与安装提示。</p>
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
              <h3>该运行时下没有容器</h3>
              <p className="muted">创建入口随 vm.* 接入开放。</p>
            </div>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>状态</th>
                  <th>环境</th>
                  <th>发起方</th>
                  <th>模式</th>
                  <th>镜像</th>
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
                      <button className="ghost" disabled title="窗口流接入后可用">
                        接管
                      </button>
                      <button className="ghost" disabled title="exec/stdio 接入后可用">
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
