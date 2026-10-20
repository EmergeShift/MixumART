import { useEffect, useMemo, useState } from 'react'

import { errorText, listContainers } from '../api'
import type { AgentMode, BackendStatus, Container, HostInfo } from '../types'

type Props = {
  backends: BackendStatus[] | null
  host: HostInfo | null
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

export function Cockpit({ backends, host }: Props) {
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
  const attributed = (containers ?? []).filter((container) => container.agent)
  const others = (containers ?? []).filter((container) => container.state !== 'running')

  return (
    <div className="cockpit">
      <section className="hero">
        <div>
          <h2>所有 Agent 的桌面入口</h2>
          <p className="muted">
            每个环境由谁发起、正在跑什么、值不值得接管——都在这一屏。MART 本身不是
            Agent；它把 {host ? `${host.os === 'macos' ? 'macOS' : host.os} 宿主` : '宿主'} 上的环境交给任意 Agent 使用。
          </p>
        </div>
        <div className="stats">
          <div className="stat">
            <strong>{ready.length}</strong>
            <span>可用运行时</span>
          </div>
          <div className="stat">
            <strong>{running.length}</strong>
            <span>运行中环境</span>
          </div>
          <div className="stat">
            <strong>{attributed.length}</strong>
            <span>Agent 归属</span>
          </div>
        </div>
      </section>

      {error && <div className="error">{error}</div>}

      <section>
        <div className="section-head">
          <div>
            <h2>运行中的 Agent 环境</h2>
            <p className="muted">卡片标注发起方与协作模式；「接管」在 computer use 接入后直接进入画面。</p>
          </div>
        </div>

        {containers === null ? (
          <p className="muted">正在读取环境…</p>
        ) : running.length === 0 ? (
          <div className="empty-panel">
            <h3>还没有运行中的环境</h3>
            <p className="muted">
              在「Agent」页选择平台与工作模式，或手动创建环境——启动入口随后端
              vm.* 接入开放。
            </p>
          </div>
        ) : (
          <div className="agent-grid">
            {running.map((container) => (
              <article className="agent-card" key={container.id}>
                <header>
                  <div className="agent-identity">
                    <span className="agent-avatar">
                      {(container.agent?.name ?? '?').slice(0, 1)}
                    </span>
                    <div>
                      <strong>{container.agent?.name ?? '未标注 Agent'}</strong>
                      <span className="muted">
                        {container.agent ? MODE_LABEL[container.agent.mode] : '来源未知'}
                        {container.agent?.core && container.agent.core !== 'none'
                          ? ` · core: ${container.agent.core}`
                          : ''}
                      </span>
                    </div>
                  </div>
                  <span className={`state state-${container.state}`}>
                    <span className="dot" />
                    {STATE_LABEL[container.state] ?? container.state}
                  </span>
                </header>

                <div className="agent-thumb">
                  <span className="thumb-hint mono">
                    {container.ports.length > 0
                      ? `wss://127.0.0.1:${container.ports[0].host}`
                      : 'no port'}
                  </span>
                  <span className="live-badge">可监看</span>
                </div>

                <div className="agent-meta">
                  <div>
                    <span className="muted">环境</span>
                    <span className="mono">{container.name}</span>
                  </div>
                  <div>
                    <span className="muted">镜像</span>
                    <span className="mono">{container.image}</span>
                  </div>
                </div>

                <footer>
                  <button className="ghost" disabled title="窗口流接入后可用">
                    接管
                  </button>
                  <button className="ghost" disabled title="exec/stdio 接入后可用">
                    终端
                  </button>
                  <button className="ghost" disabled title="事件流接入后可用">
                    日志
                  </button>
                </footer>
              </article>
            ))}
          </div>
        )}
      </section>

      {others.length > 0 && (
        <section>
          <div className="section-head">
            <div>
              <h2>其他环境</h2>
              <p className="muted">已停止、暂停或尚未启动的容器。</p>
            </div>
          </div>
          <div className="row-list">
            {others.map((container) => (
              <div className="row-item" key={container.id}>
                <span className={`state state-${container.state}`}>
                  <span className="dot" />
                  {STATE_LABEL[container.state] ?? container.state}
                </span>
                <span className="mono">{container.name}</span>
                <span className="muted">{container.agent?.name ?? '未标注 Agent'}</span>
                <span className="mono muted">{container.image}</span>
              </div>
            ))}
          </div>
        </section>
      )}
    </div>
  )
}
