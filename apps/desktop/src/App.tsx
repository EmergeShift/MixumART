import { useCallback, useEffect, useState } from 'react'

import { errorText, hostInfo, runtimeStatus } from './api'
import { isDemo } from './demo'
import { statusLabel, statusTone } from './status'
import { Agents } from './views/Agents'
import { Cockpit } from './views/Cockpit'
import { Containers } from './views/Containers'
import { Settings } from './views/Settings'
import { Surfaces } from './views/Surfaces'
import type { BackendStatus, HostInfo } from './types'

type Tab = 'cockpit' | 'containers' | 'surfaces' | 'agents' | 'settings'

const OS_LABELS: Record<string, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
}

const NAV: { tab: Tab; label: string; hint: string }[] = [
  { tab: 'cockpit', label: '总览', hint: '所有 Agent 与它们的环境' },
  { tab: 'containers', label: '容器', hint: '逐个环境的终端、程序与状态' },
  { tab: 'surfaces', label: '窗口', hint: '监看与接管 computer use' },
  { tab: 'agents', label: 'Agent', hint: '平台接入、工作模式与 Agent Core' },
  { tab: 'settings', label: '设置', hint: 'mixumOS 与 MART 配置' },
]

export default function App() {
  const [tab, setTab] = useState<Tab>('cockpit')
  const [host, setHost] = useState<HostInfo | null>(null)
  const [backends, setBackends] = useState<BackendStatus[] | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    setRefreshing(true)
    setError(null)
    try {
      const [nextHost, nextBackends] = await Promise.all([hostInfo(), runtimeStatus()])
      setHost(nextHost)
      setBackends(nextBackends)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setRefreshing(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const current = NAV.find((item) => item.tab === tab)!
  const hostMeta = host
    ? [OS_LABELS[host.os] ?? host.os, host.arch, `v${host.appVersion}`].join(' · ')
    : '正在读取宿主信息…'

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">◈</span>
          <div>
            mixum <span>ART</span>
            <small>Agent Env</small>
          </div>
        </div>

        <nav className="nav">
          {NAV.map((item) => (
            <button
              key={item.tab}
              className={tab === item.tab ? 'active' : ''}
              onClick={() => setTab(item.tab)}
              title={item.hint}
            >
              <span className="nav-label">{item.label}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar-foot">
          <div className="runtime-pills">
            {(backends ?? []).map((backend) => (
              <span className={`pill ${statusTone(backend.status)}`} key={backend.kind}>
                <span className="dot" />
                {backend.name.replace(' Container', '')}
                <em>{statusLabel(backend.status)}</em>
              </span>
            ))}
          </div>
          <div className="version">{host ? `MART v${host.appVersion}` : '—'}</div>
        </div>
      </aside>

      <div className="main">
        <header className="topbar">
          <div>
            <h1>{current.label}</h1>
            <span className="muted">{current.hint}</span>
          </div>
          <div className="topbar-right">
            {isDemo() && <span className="demo-badge">演示数据</span>}
            <span className="muted mono">{hostMeta}</span>
            <button className="ghost" onClick={() => void refresh()} disabled={refreshing}>
              {refreshing ? '刷新中…' : '刷新'}
            </button>
          </div>
        </header>

        <main className="content">
          {error && <div className="error">{error}</div>}
          {tab === 'cockpit' && <Cockpit backends={backends} host={host} />}
          {tab === 'containers' && <Containers backends={backends} />}
          {tab === 'surfaces' && <Surfaces />}
          {tab === 'agents' && <Agents />}
          {tab === 'settings' && (
            <Settings backends={backends} host={host} onRefresh={() => void refresh()} />
          )}
        </main>
      </div>
    </div>
  )
}
