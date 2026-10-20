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

const NAV: { tab: Tab; label: string; sub: string }[] = [
  { tab: 'cockpit', label: '首页', sub: '这台电脑上的所有 Agent 环境' },
  { tab: 'containers', label: '环境', sub: '每个环境在哪、由谁使用' },
  { tab: 'surfaces', label: '窗口', sub: '监看与接管环境里的应用' },
  { tab: 'agents', label: 'Agent', sub: '对接外部 Agent，决定谁在环境里干活' },
  { tab: 'settings', label: '设置', sub: '运行软件、环境系统与应用设置' },
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
    : '读取中…'

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <svg width="26" height="26" viewBox="0 0 26 26" aria-hidden="true">
            <rect x="1.5" y="1.5" width="23" height="23" rx="6" fill="none" stroke="#2f5a7a" />
            <path
              d="M7 17.5V9l6 5 6-5v8.5"
              fill="none"
              stroke="#2f5a7a"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          <div className="brand-text">
            <strong>mixum ART</strong>
            <span>Agent Env</span>
          </div>
        </div>

        <nav className="nav">
          {NAV.map((item) => (
            <button
              key={item.tab}
              className={tab === item.tab ? 'active' : ''}
              onClick={() => setTab(item.tab)}
            >
              {item.label}
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
            <span className="sub">{current.sub}</span>
          </div>
          <div className="topbar-right">
            {isDemo() && <span className="demo-badge">演示数据</span>}
            <span className="host-meta">{hostMeta}</span>
            <button className="ghost" onClick={() => void refresh()} disabled={refreshing}>
              {refreshing ? '刷新中…' : '刷新'}
            </button>
          </div>
        </header>

        <main className="content">
          {error && <div className="error">{error}</div>}
          {tab === 'cockpit' && <Cockpit backends={backends} />}
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
