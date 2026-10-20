import { useEffect, useMemo, useState } from 'react'

import { errorText, listContainers } from '../api'
import type { AgentMode, BackendStatus, Container } from '../types'

type Props = {
  backends: BackendStatus[] | null
}

const MODE_LABEL: Record<AgentMode, string> = {
  'as-tool': '直接使用',
  'sub-agent': '派发任务',
  isolated: '完全隔离',
}

export function Cockpit({ backends }: Props) {
  const ready = useMemo(
    () => (backends ?? []).filter((backend) => backend.status.state === 'ready'),
    [backends],
  )
  const [containers, setContainers] = useState<Container[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    if (ready.length === 0) {
      if (backends !== null) setContainers([])
      return
    }
    void (async () => {
      try {
        const lists = await Promise.all(ready.map((backend) => listContainers(backend.kind)))
        if (!cancelled) setContainers(lists.flat())
      } catch (err) {
        if (!cancelled) {
          setError(errorText(err))
          setContainers([])
        }
      }
    })()
    return () => {
      cancelled = true
    }
  }, [ready, backends])

  const running = (containers ?? []).filter((container) => container.state === 'running')
  const others = (containers ?? []).filter((container) => container.state !== 'running')

  return (
    <div>
      {error && <div className="error">{error}</div>}

      {containers === null ? (
        <p className="muted">正在读取…</p>
      ) : (
        <>
          <p className="summary-line">
            <strong>{running.length}</strong>
            个环境正在运行，共 {containers.length} 个
          </p>

          <section>
            <div className="section-head">
              <div>
                <h2>正在运行</h2>
                <p>每一行是一个环境。右侧可以直接监看画面或进入终端。</p>
              </div>
            </div>

            {running.length === 0 ? (
              <div className="empty-panel">
                <h3>现在没有正在运行的环境</h3>
                <p>在「Agent」页选择接入方式，或先在「环境」页查看已有环境。</p>
              </div>
            ) : (
              <div className="env-list">
                {running.map((container) => (
                  <div className="env-row" key={container.id}>
                    <span className="state state-running" title="正在运行">
                      <span className="dot" />
                    </span>
                    <div className="env-name">
                      <strong>{container.name}</strong>
                      <span>{container.id.slice(0, 12)}</span>
                    </div>
                    <div className="env-cell">
                      {container.agent ? (
                        <>
                          {container.agent.name}
                          <small>{MODE_LABEL[container.agent.mode]}</small>
                        </>
                      ) : (
                        <span className="muted">—</span>
                      )}
                    </div>
                    <div className="env-cell">
                      <span className="mono">{container.image}</span>
                      <small>
                        {container.ports.length > 0
                          ? `端口 ${container.ports.map((port) => port.host).join(', ')}`
                          : '无端口'}
                      </small>
                    </div>
                    <div className="env-actions">
                      <button className="ghost" disabled title="画面接入后可用">
                        监看
                      </button>
                      <button className="ghost" disabled title="终端接入后可用">
                        终端
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>

          {others.length > 0 && (
            <section>
              <div className="section-head">
                <div>
                  <h2>已停止或暂停</h2>
                  <p>这些环境仍保留，随时可以继续。</p>
                </div>
              </div>
              <div className="env-list">
                {others.map((container) => (
                  <div className="env-row" key={container.id}>
                    <span
                      className={`state state-${container.state}`}
                      title={container.state === 'paused' ? '已暂停' : '已停止'}
                    >
                      <span className="dot" />
                    </span>
                    <div className="env-name">
                      <strong>{container.name}</strong>
                      <span>{container.id.slice(0, 12)}</span>
                    </div>
                    <div className="env-cell">
                      {container.agent ? (
                        <>
                          {container.agent.name}
                          <small>{container.state === 'paused' ? '已暂停' : '已停止'}</small>
                        </>
                      ) : (
                        <span className="muted">—</span>
                      )}
                    </div>
                    <div className="env-cell">
                      <span className="mono">{container.image}</span>
                    </div>
                    <div className="env-actions">
                      <button className="ghost" disabled title="环境管理接入后可用">
                        继续
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}
        </>
      )}
    </div>
  )
}
