import { useCallback, useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

import logo from './assets/logo.png'
import { errorText, hostInfo, listApprovals, listPendingPrivileges, runtimeStatus, type ApprovalView, type PendingPrivilege } from './api'
import { isDemo } from './demo'
import { statusLabel, statusTone } from './status'
import { ThemeToggle, WindowControls } from './components/Chrome'
import { PrivilegeDialog, type PrivilegeEvent } from './components/PrivilegeDialog'
import { ApprovalCenter } from './components/ApprovalCenter'
import { Icon, type IconName } from './components/Icons'
import { Agents } from './views/Agents'
import { Cockpit } from './views/Cockpit'
import { Containers } from './views/Containers'
import { Settings } from './views/Settings'
import { Surfaces } from './views/Surfaces'
import type { BackendStatus, HostInfo } from './types'
import { detectPlatform } from './platform'
import {
  applyTheme,
  getStoredThemeMode,
  resolveTheme,
  storeThemeMode,
  type ResolvedTheme,
  type ThemeMode,
} from './theme'

type Tab = 'cockpit' | 'containers' | 'surfaces' | 'agents' | 'settings'

const OS_LABELS: Record<string, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
}

const PLATFORM = detectPlatform()

const NAV: { tab: Tab; label: string; sub: string; icon: IconName }[] = [
  { tab: 'cockpit', label: '首页', sub: '这台电脑上的所有 Agent 环境', icon: 'home' },
  { tab: 'containers', label: '环境', sub: '每个环境在哪、由谁使用', icon: 'layers' },
  { tab: 'surfaces', label: '窗口', sub: '监看与接管环境里的应用', icon: 'monitor' },
  { tab: 'agents', label: 'Agent', sub: '对接外部 Agent，决定谁在环境里干活', icon: 'agent' },
  { tab: 'settings', label: '设置', sub: '运行软件、环境系统与应用设置', icon: 'settings' },
]

export default function App() {
  const [tab, setTab] = useState<Tab>('cockpit')
  const [host, setHost] = useState<HostInfo | null>(null)
  const [backends, setBackends] = useState<BackendStatus[] | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [privilege, setPrivilege] = useState<PrivilegeEvent | null>(null)
  const [approvals, setApprovals] = useState<ApprovalView[]>([])
  const [sudoRequests, setSudoRequests] = useState<PendingPrivilege[]>([])
  const [showApprovals, setShowApprovals] = useState(false)
  const [themeMode, setThemeMode] = useState<ThemeMode>(getStoredThemeMode)
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveTheme(getStoredThemeMode()),
  )

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

  useEffect(() => {
    document.documentElement.dataset.platform = PLATFORM
  }, [])

  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const sync = () => {
      const next = themeMode === 'system'
        ? (media.matches ? 'dark' : 'light')
        : themeMode
      applyTheme(next)
      setResolvedTheme(next)
    }
    sync()
    media.addEventListener('change', sync)
    return () => media.removeEventListener('change', sync)
  }, [themeMode])

  useEffect(() => {
    let dispose: (() => void) | undefined
    listen<PrivilegeEvent>('privilege-request', (event) => setPrivilege(event.payload))
      .then((unlisten) => { dispose = unlisten })
      .catch(() => undefined)
    return () => dispose?.()
  }, [])

  const refreshApprovals = useCallback(() => {
    listApprovals()
      .then(setApprovals)
      .catch(() => undefined)
    listPendingPrivileges()
      .then(setSudoRequests)
      .catch(() => undefined)
  }, [])

  useEffect(() => {
    refreshApprovals()
    const timer = setInterval(refreshApprovals, 5000)
    return () => clearInterval(timer)
  }, [refreshApprovals])

  const toggleTheme = useCallback(() => {
    setThemeMode((previous) => {
      const next: ThemeMode = resolveTheme(previous) === 'dark' ? 'light' : 'dark'
      storeThemeMode(next)
      return next
    })
  }, [])

  const current = NAV.find((item) => item.tab === tab)!
  const isHome = tab === 'cockpit'
  const hostMeta = host
    ? [OS_LABELS[host.os] ?? host.os, host.arch, `v${host.appVersion}`].join(' · ')
    : '读取中…'

  const chromeActions = (
    <div className="chrome-actions">
      {approvals.length + sudoRequests.length > 0 && (
        <button type="button" className="btn ghost" onClick={() => setShowApprovals(true)}>
          审批 {approvals.length + sudoRequests.length}
        </button>
      )}
      {isDemo() && <span className="demo-badge">演示数据</span>}
      <span className="host-meta">{hostMeta}</span>
      <button
        type="button"
        className="icon-btn"
        onClick={() => void refresh()}
        disabled={refreshing}
        title="刷新"
        aria-label="刷新"
      >
        <Icon name="refresh" size={15} className={refreshing ? 'spin' : undefined} />
      </button>
      <ThemeToggle theme={resolvedTheme} onToggle={toggleTheme} />
      {PLATFORM === 'windows' && <WindowControls />}
    </div>
  )

  return (
    <div className="app">
      <div className="app-body">
        <aside className="sidebar">
          <div className="sidebar-drag" data-tauri-drag-region="deep" aria-hidden="true" />

          <div className="brand">
            <img src={logo} alt="mixum ART" className="brand-logo" />
            <div className="brand-text">
              <strong>mixum ART</strong>
              <span>Agent Runtime</span>
            </div>
          </div>

          <nav className="nav">
            {NAV.map((item) => (
              <button
                key={item.tab}
                type="button"
                className={tab === item.tab ? 'active' : ''}
                onClick={() => setTab(item.tab)}
                aria-current={tab === item.tab ? 'page' : undefined}
              >
                <Icon name={item.icon} size={15} />
                <span>{item.label}</span>
              </button>
            ))}
          </nav>

          <div className="sidebar-foot">
            <div className="runtime-pills">
              {backends === null ? (
                <span className="pill">检测运行软件…</span>
              ) : (
                backends.map((backend) => (
                  <span className={`pill ${statusTone(backend.status)}`} key={backend.kind}>
                    <span className="dot" />
                    {backend.name.replace(' Container', '')}
                    <em>{statusLabel(backend.status)}</em>
                  </span>
                ))
              )}
            </div>
            <div className="version">{host ? `MART v${host.appVersion}` : '—'}</div>
          </div>
        </aside>

        <main className="content-frame">
          <div className="panel-header" data-tauri-drag-region="deep">
            <div className="panel-header-left">
              {isHome ? (
                <span className="panel-crumb">首页</span>
              ) : (
                <>
                  <span className="panel-title">{current.label}</span>
                  <span className="panel-sub">{current.sub}</span>
                </>
              )}
            </div>
            {chromeActions}
          </div>

          <div className={`content ${isHome ? 'content-home' : 'content-page'}`}>
            {error && <div className="error">{error}</div>}
            {tab === 'cockpit' && (
              <Cockpit backends={backends} onOpenEnvironments={() => setTab('containers')} />
            )}
            {tab === 'containers' && <Containers backends={backends} />}
            {tab === 'surfaces' && <Surfaces backends={backends} onOpenEnvironments={() => setTab('containers')} />}
            {tab === 'agents' && <Agents />}
            {tab === 'settings' && (
              <Settings
                backends={backends}
                host={host}
                appearance={
                  themeMode === 'system'
                    ? '跟随系统'
                    : resolvedTheme === 'dark' ? '深色' : '浅色'
                }
                onRefresh={() => void refresh()}
              />
            )}
          </div>
        </main>
      </div>
      {privilege && (
        <PrivilegeDialog event={privilege} onClose={() => setPrivilege(null)} />
      )}
      {showApprovals && (
        <ApprovalCenter
          approvals={approvals}
          sudoRequests={sudoRequests}
          onClose={() => setShowApprovals(false)}
          onChanged={refreshApprovals}
        />
      )}
    </div>
  )
}
