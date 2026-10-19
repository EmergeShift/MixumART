import { useCallback, useEffect, useState } from 'react'

import { errorText, hostInfo, runtimeStatus } from './api'
import { Containers } from './views/Containers'
import { Environments } from './views/Environments'
import { Surfaces } from './views/Surfaces'
import type { BackendStatus, HostInfo } from './types'

type Tab = 'environments' | 'containers' | 'surfaces'

const OS_LABELS: Record<string, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
}

export default function App() {
  const [tab, setTab] = useState<Tab>('environments')
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

  const title =
    tab === 'environments' ? '宿主机环境' : tab === 'containers' ? '容器' : '窗口'
  const hostMeta = host
    ? [OS_LABELS[host.os] ?? host.os, host.arch, `v${host.appVersion}`].join(' · ')
    : '正在读取宿主信息…'

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          mixum <span>Agent Env</span>
        </div>
        <nav className="nav">
          <button
            className={tab === 'environments' ? 'active' : ''}
            onClick={() => setTab('environments')}
          >
            环境
          </button>
          <button
            className={tab === 'containers' ? 'active' : ''}
            onClick={() => setTab('containers')}
          >
            容器
          </button>
          <button
            className={tab === 'surfaces' ? 'active' : ''}
            onClick={() => setTab('surfaces')}
          >
            窗口
          </button>
        </nav>
        <div className="sidebar-foot">{host ? `v${host.appVersion}` : '—'}</div>
      </aside>

      <div className="main">
        <header className="topbar">
          <h1>{title}</h1>
          <span className="muted">{hostMeta}</span>
        </header>

        <main className="content">
          {error && <div className="error">{error}</div>}
          {tab === 'environments' ? (
            <Environments
              backends={backends}
              refreshing={refreshing}
              onRefresh={() => void refresh()}
            />
          ) : tab === 'containers' ? (
            <Containers backends={backends} />
          ) : (
            <Surfaces />
          )}
        </main>
      </div>
    </div>
  )
}
