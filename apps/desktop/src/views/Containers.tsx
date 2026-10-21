import { useEffect, useMemo, useState } from 'react'

import {
  errorText,
  listContainers,
  listSurfaces,
  loadEnvironmentConnections,
  openSurfaceWindow,
  removeEnvironmentConnection,
  saveEnvironmentConnection,
} from '../api'
import type {
  AgentMode,
  BackendStatus,
  Container,
  EnvironmentConnection,
  RuntimeKind,
  SurfaceInfo,
} from '../types'

type Props = {
  backends: BackendStatus[] | null
}

type ConnectionDraft = {
  addr: string
  token: string
  fingerprint: string
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

const defaultAddress = (container: Container) => {
  const port =
    container.ports.find((item) => item.container === 7601) ?? container.ports[0]
  return port ? `127.0.0.1:${port.host}` : '127.0.0.1:7601'
}

export function Containers({ backends }: Props) {
  const ready = useMemo(
    () => (backends ?? []).filter((backend) => backend.status.state === 'ready'),
    [backends],
  )
  const [kind, setKind] = useState<RuntimeKind | null>(null)
  const [containers, setContainers] = useState<Container[] | null>(null)
  const [connections, setConnections] = useState<Record<string, EnvironmentConnection>>({})
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [editor, setEditor] = useState<Container | null>(null)
  const [draft, setDraft] = useState<ConnectionDraft>({
    addr: '',
    token: '',
    fingerprint: '',
  })
  const [browser, setBrowser] = useState<Container | null>(null)
  const [surfaces, setSurfaces] = useState<SurfaceInfo[] | null>(null)
  const [surfaceError, setSurfaceError] = useState<string | null>(null)
  const [surfaceLoading, setSurfaceLoading] = useState(false)

  useEffect(() => {
    if (kind === null && ready.length > 0) setKind(ready[0].kind)
  }, [kind, ready])

  useEffect(() => {
    void loadEnvironmentConnections()
      .then(setConnections)
      .catch((err: unknown) => setError(errorText(err)))
  }, [])

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

  const openEditor = (container: Container) => {
    const existing = connections[container.id]
    setEditor(container)
    setBrowser(null)
    setDraft({
      addr: existing?.addr ?? defaultAddress(container),
      token: existing?.token ?? '',
      fingerprint: existing?.fingerprint ?? '',
    })
  }

  const browse = async (container: Container, profile?: EnvironmentConnection) => {
    const connection = profile ?? connections[container.id]
    if (!connection) {
      openEditor(container)
      return
    }
    setEditor(null)
    setBrowser(container)
    setSurfaces(null)
    setSurfaceError(null)
    setSurfaceLoading(true)
    try {
      setSurfaces(
        await listSurfaces(
          connection.addr,
          connection.token,
          connection.fingerprint ?? undefined,
        ),
      )
    } catch (err) {
      setSurfaceError(errorText(err))
      setSurfaces([])
    } finally {
      setSurfaceLoading(false)
    }
  }

  const saveAndBrowse = async () => {
    if (!editor) return
    if (!draft.addr.trim() || !draft.token.trim()) {
      setSurfaceError('节点地址和访问令牌都不能为空')
      return
    }
    const connection: EnvironmentConnection = {
      addr: draft.addr.trim(),
      token: draft.token.trim(),
      fingerprint: draft.fingerprint.trim() || null,
    }
    try {
      await saveEnvironmentConnection(editor.id, connection)
      setConnections((previous) => ({ ...previous, [editor.id]: connection }))
      await browse(editor, connection)
    } catch (err) {
      setSurfaceError(errorText(err))
    }
  }

  const removeConnection = async (container: Container) => {
    try {
      await removeEnvironmentConnection(container.id)
      setConnections((previous) => {
        const next = { ...previous }
        delete next[container.id]
        return next
      })
      setEditor(null)
      setBrowser(null)
      setSurfaces(null)
    } catch (err) {
      setSurfaceError(errorText(err))
    }
  }

  const openSurface = async (surface: SurfaceInfo) => {
    if (!browser) return
    const connection = connections[browser.id]
    if (!connection) return
    setSurfaceError(null)
    try {
      await openSurfaceWindow(
        connection.addr,
        connection.token,
        connection.fingerprint ?? undefined,
        surface,
      )
    } catch (err) {
      setSurfaceError(errorText(err))
    }
  }

  return (
    <section>
      <div className="section-head">
        <div>
          <h2>全部环境</h2>
          <p>连接环境后可以打开它的窗口；暂停、继续与终端入口会随环境管理功能上线。</p>
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

          {editor && (
            <section className="connect-panel" aria-label={`连接 ${editor.name}`}>
              <div className="section-head">
                <div>
                  <h3>连接 {editor.name}</h3>
                  <p>连接信息会保存在本机应用数据目录，不会写进快照。</p>
                </div>
                <button className="ghost" onClick={() => setEditor(null)}>
                  取消
                </button>
              </div>
              <div className="connect-grid">
                <div className="field">
                  <label htmlFor="container-map-addr">节点地址</label>
                  <input
                    id="container-map-addr"
                    className="mono"
                    value={draft.addr}
                    onChange={(event) => setDraft({ ...draft, addr: event.target.value })}
                    placeholder="例如 127.0.0.1:7601"
                    spellCheck={false}
                  />
                </div>
                <div className="field">
                  <label htmlFor="container-map-token">访问令牌</label>
                  <input
                    id="container-map-token"
                    className="mono"
                    type="password"
                    value={draft.token}
                    onChange={(event) => setDraft({ ...draft, token: event.target.value })}
                    placeholder="map1.…"
                    spellCheck={false}
                  />
                </div>
                <div className="field">
                  <label htmlFor="container-map-fp">证书指纹（可选）</label>
                  <input
                    id="container-map-fp"
                    className="mono"
                    value={draft.fingerprint}
                    onChange={(event) => setDraft({ ...draft, fingerprint: event.target.value })}
                    placeholder="首次连接可留空"
                    spellCheck={false}
                  />
                </div>
              </div>
              {surfaceError && <div className="error">{surfaceError}</div>}
              <div className="connect-actions">
                <button className="primary" onClick={() => void saveAndBrowse()}>
                  保存并查看窗口
                </button>
                <button className="ghost" onClick={() => void removeConnection(editor)}>
                  移除连接档案
                </button>
              </div>
            </section>
          )}

          {browser && (
            <section className="connect-panel" aria-label={`${browser.name} 的窗口`}>
              <div className="section-head">
                <div>
                  <h3>{browser.name} 的窗口</h3>
                  <p className="mono muted">{connections[browser.id]?.addr}</p>
                </div>
                <button className="ghost" onClick={() => setBrowser(null)}>
                  关闭
                </button>
              </div>
              {surfaceError && <div className="error">{surfaceError}</div>}
              {surfaceLoading || surfaces === null ? (
                <p className="muted">正在查找窗口…</p>
              ) : surfaces.length === 0 ? (
                <p className="muted">这个环境里暂时没有窗口。</p>
              ) : (
                <table className="table">
                  <thead>
                    <tr>
                      <th>窗口</th>
                      <th>应用</th>
                      <th>尺寸</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {surfaces.map((surface) => (
                      <tr key={surface.id}>
                        <td>{surface.title || '未命名窗口'}</td>
                        <td className="mono muted">{surface.appId || '—'}</td>
                        <td className="mono">
                          {surface.width}×{surface.height}
                        </td>
                        <td className="row-actions">
                          <button className="ghost" onClick={() => void openSurface(surface)}>
                            打开窗口
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
          )}

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
                {containers.map((container) => {
                  const connection = connections[container.id]
                  return (
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
                          <span className="mono muted">
                            {connection ? connection.addr : container.id.slice(0, 12)}
                          </span>
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
                          : container.ports
                              .map((port) => `${port.host}→${port.container}`)
                              .join(', ')}
                      </td>
                      <td className="row-actions">
                        <button
                          className="ghost"
                          onClick={() =>
                            connection ? void browse(container) : openEditor(container)
                          }
                        >
                          {connection ? '监看' : '连接'}
                        </button>
                        <button className="ghost" disabled title="终端接入后可用">
                          终端
                        </button>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
        </>
      )}
    </section>
  )
}
