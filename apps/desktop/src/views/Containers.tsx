import { useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

import {
  closeContainerTerminal,
  connectEnvironment,
  containerSpec,
  containerTerminalInput,
  createEnvironment,
  errorText,
  listContainers,
  listEnvironmentSurfaces,
  listMigrations,
  migrateSnapshot,
  migrateSnapshotNew,
  type MigrationFile,
  loadEnvironmentConnections,
  openContainerTerminal,
  openEnvironmentSurface,
  runtimeLogs,
  removeEnvironment,
  removeEnvironmentConnection,
  saveEnvironmentConnection,
  snapshotCreateKind,
  snapshotExport,
  snapshotImport,
  snapshotRestore,
  snapshotTree,
  startEnvironment,
  stopEnvironment,
} from '../api'
import type {
  AgentMode,
  BackendStatus,
  ConnectionInfo,
  Container,
  EnvironmentConnection,
  RuntimeKind,
  SnapshotTree,
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

/** 只有 mixumOS 镜像里才有 agentd/MAP；普通容器只能开终端。 */
const isMapImage = (image: string) => image.toLowerCase().includes('mixumos')

export function Containers({ backends }: Props) {
  const ready = useMemo(
    () => (backends ?? []).filter((backend) => backend.status.state === 'ready'),
    [backends],
  )
  const [kind, setKind] = useState<RuntimeKind | null>(null)
  const [containers, setContainers] = useState<Container[] | null>(null)
  const [connections, setConnections] = useState<Record<string, ConnectionInfo>>({})
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
  const [creating, setCreating] = useState(false)
  const [createDraft, setCreateDraft] = useState({ name: 'mixum-node', image: '', hostPort: '17601' })
  const [actionId, setActionId] = useState<string | null>(null)
  const [snapshots, setSnapshots] = useState<SnapshotTree | null>(null)
  const [snapshotError, setSnapshotError] = useState<string | null>(null)
  const [snapshotLoading, setSnapshotLoading] = useState(false)
  const [snapshotLabel, setSnapshotLabel] = useState('')
  const [snapshotKind, setSnapshotKind] = useState('disk')
  const [snapshotAction, setSnapshotAction] = useState<string | null>(null)
  const [migrations, setMigrations] = useState<MigrationFile[]>([])
  const [migrationPath, setMigrationPath] = useState('')
  const [migrationNotice, setMigrationNotice] = useState('')
  const [migrationTarget, setMigrationTarget] = useState('')
  const [terminal, setTerminal] = useState<Container | null>(null)
  const [terminalId, setTerminalId] = useState<number | null>(null)
  const [terminalLines, setTerminalLines] = useState<string[]>([])
  const [terminalCommand, setTerminalCommand] = useState('')
  const [terminalRunning, setTerminalRunning] = useState(false)
  const [terminalError, setTerminalError] = useState<string | null>(null)

  const canManage = kind !== null

  useEffect(() => {
    if (kind === null && ready.length > 0) setKind(ready[0].kind)
  }, [kind, ready])

  useEffect(() => {
    void loadEnvironmentConnections()
      .then(setConnections)
      .catch((err: unknown) => setError(errorText(err)))
  }, [])

  useEffect(() => {
    if (terminalId === null) return
    let disposed = false
    const unlisten = listen<{ terminal: number; data: string }>('terminal-data', (event) => {
      if (disposed || event.payload.terminal !== terminalId) return
      setTerminalLines((previous) => [...previous, event.payload.data])
    })
    return () => {
      disposed = true
      void unlisten.then((stop) => stop())
    }
  }, [terminalId])

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

  const loadSnapshots = async (container: Container) => {
    setSnapshotLoading(true)
    setSnapshotError(null)
    try {
      setSnapshots((await snapshotTree(container.id)) as SnapshotTree)
    } catch (err) {
      setSnapshotError(errorText(err))
      setSnapshots(null)
    } finally {
      setSnapshotLoading(false)
    }
  }

  const browse = async (container: Container) => {
    setEditor(null)
    setTerminal(null)
    setBrowser(container)
    setSurfaces(null)
    setSurfaceError(null)
    setSurfaceLoading(true)
    void loadSnapshots(container)
    try {
      setSurfaces(await listEnvironmentSurfaces(container.id))
    } catch (err) {
      setSurfaceError(errorText(err))
      setSurfaces([])
    } finally {
      setSurfaceLoading(false)
    }
  }

  const autoConnect = async () => {
    if (!editor || !kind) return
    setSurfaceError(null)
    try {
      const info = await connectEnvironment(kind, editor.id, draft.addr.trim() || defaultAddress(editor))
      setConnections((previous) => ({ ...previous, [editor.id]: info }))
      await browse(editor)
    } catch (err) {
      setSurfaceError(errorText(err))
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
      setConnections((previous) => ({
        ...previous,
        [editor.id]: { addr: connection.addr, fingerprint: connection.fingerprint },
      }))
      await browse(editor)
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
      await openEnvironmentSurface(browser.id, surface)
    } catch (err) {
      setSurfaceError(errorText(err))
    }
  }

  const create = async () => {
    if (!kind || !createDraft.name.trim() || !createDraft.image.trim()) {
      setError('名称和 rootfs tar 路径都不能为空')
      return
    }
    setActionId('create')
    setError(null)
    try {
      await createEnvironment(kind, {
        name: createDraft.name.trim(),
        image: createDraft.image.trim(),
        ports:
          kind === 'wsl2'
            ? []
            : [{ host: Number(createDraft.hostPort) || 17601, container: 7601, protocol: 'tcp' }],
        mounts: [],
        env: {},
        command: [],
      })
      setCreating(false)
      await load()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setActionId(null)
    }
  }

  const runAction = async (container: Container, action: 'start' | 'stop' | 'remove') => {
    if (!kind) return
    setActionId(container.id)
    setError(null)
    try {
      if (action === 'start') await startEnvironment(kind, container.id)
      else if (action === 'stop') await stopEnvironment(kind, container.id)
      else {
        if (!window.confirm(`删除环境 ${container.name}？`)) return
        await removeEnvironment(kind, container.id)
      }
      await load()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setActionId(null)
    }
  }

  const openManual = (container: Container) => {
    setEditor(container)
    setBrowser(null)
    setDraft({ addr: defaultAddress(container), token: '', fingerprint: '' })
  }

  const openEnvironment = async (container: Container) => {
    if (!kind) return
    if (!isMapImage(container.image)) {
      setSurfaceError('这是普通容器，没有 mixumOS agentd/MAP；只能打开终端，不能查看窗口。')
      setBrowser(container)
      setSurfaces([])
      return
    }
    setActionId(container.id)
    setSurfaceError(null)
    try {
      if (container.state !== 'running') {
        await startEnvironment(kind, container.id)
        await new Promise((resolve) => setTimeout(resolve, 700))
        await load()
      }
      let profile = connections[container.id]
      try {
        profile = await connectEnvironment(
          kind,
          container.id,
          defaultAddress(container),
        )
        setConnections((previous) => ({ ...previous, [container.id]: profile! }))
      } catch (autoError) {
        if (!profile) throw autoError
      }
      await browse(container)
    } catch (err) {
      setSurfaceError(
        `${errorText(err)}。只有带 mixumOS agentd 的容器能提供 MAP 窗口；普通容器请用终端。`,
      )
      setBrowser(container)
      setSurfaces([])
    } finally {
      setActionId(null)
    }
  }

  const openTerminal = async (container: Container) => {
    if (!kind) return
    if (!localStorage.getItem('mixum.terminal.ack')) {
      const accepted = window.confirm(
        '终端会以本机用户身份进入容器，拥有该环境的完全控制权；不是 MAP 监看权限。继续？',
      )
      if (!accepted) return
      localStorage.setItem('mixum.terminal.ack', '1')
    }
    setBrowser(null)
    setEditor(null)
    setTerminal(container)
    setTerminalLines([])
    setTerminalCommand('')
    setTerminalError(null)
    try {
      const id = await openContainerTerminal(kind, container.id)
      setTerminalId(id)
    } catch (err) {
      setTerminalError(errorText(err))
    }
  }

  const runCommand = async () => {
    if (terminalId === null || !terminalCommand.trim()) return
    const command = terminalCommand.trim()
    setTerminalCommand('')
    try {
      await containerTerminalInput(terminalId, `${command}\n`)
    } catch (err) {
      setTerminalError(errorText(err))
    }
  }

  const loadLogs = async () => {
    if (!terminal || !kind) return
    setTerminalRunning(true)
    setTerminalError(null)
    try {
      const lines = await runtimeLogs(kind, terminal.id)
      setTerminalLines((previous) => [...previous, '--- 环境日志 ---', ...lines])
    } catch (err) {
      setTerminalError(errorText(err))
    } finally {
      setTerminalRunning(false)
    }
  }

  const closeTerminal = async () => {
    if (terminalId !== null) {
      try {
        await closeContainerTerminal(terminalId)
      } catch (err) {
        setTerminalError(errorText(err))
      }
    }
    setTerminalId(null)
    setTerminal(null)
  }

  const createSnapshot = async () => {
    if (!browser) return
    setSnapshotAction('create')
    setSnapshotError(null)
    try {
      await snapshotCreateKind(browser.id, snapshotKind, snapshotLabel)
      setSnapshotLabel('')
      await loadSnapshots(browser)
    } catch (err) {
      setSnapshotError(errorText(err))
    } finally {
      setSnapshotAction(null)
    }
  }

  const restoreSnapshot = async (node: string) => {
    if (!browser) return
    if (!window.confirm(`回退到快照 ${node}？当前工作区会被替换。`)) return
    setSnapshotAction(node)
    setSnapshotError(null)
    try {
      await snapshotRestore(browser.id, node)
      await loadSnapshots(browser)
    } catch (err) {
      setSnapshotError(errorText(err))
    } finally {
      setSnapshotAction(null)
    }
  }

  const exportSnapshot = async (node: string) => {
    if (!browser) return
    setSnapshotAction(`export:${node}`)
    setSnapshotError(null)
    setMigrationNotice('')
    try {
      let inspected = null
      if (kind) {
        try {
          inspected = await containerSpec(kind, browser.id)
        } catch {
          inspected = null
        }
      }
      const file = await snapshotExport(
        browser.id,
        node,
        inspected?.spec ?? null,
        inspected?.imageDigest ?? null,
        kind ?? undefined,
        snapshots?.branch ?? undefined,
      )
      setMigrationNotice(`已导出 ${file.path}（${file.size} 字节）`)
      setMigrations(await listMigrations())
    } catch (err) {
      setSnapshotError(errorText(err))
    } finally {
      setSnapshotAction(null)
    }
  }

  const refreshMigrations = async () => {
    try {
      setMigrations(await listMigrations())
    } catch (err) {
      setSnapshotError(errorText(err))
    }
  }

  const importMigration = async () => {
    if (!browser || !migrationPath) return
    if (!window.confirm(`把 ${migrationPath} 导入当前环境？会新增一个快照节点。`)) return
    setSnapshotAction('import')
    setSnapshotError(null)
    setMigrationNotice('')
    try {
      const node = await snapshotImport(browser.id, migrationPath)
      setMigrationNotice(`已导入节点 ${node}`)
      await loadSnapshots(browser)
    } catch (err) {
      setSnapshotError(errorText(err))
    } finally {
      setSnapshotAction(null)
    }
  }

  const migrateToTarget = async (node: string) => {
    if (!browser || !migrationTarget) return
    if (!window.confirm(`把快照 ${node} 迁移到所选环境？`)) return
    setSnapshotAction(`migrate:${node}`)
    setSnapshotError(null)
    setMigrationNotice('')
    try {
      const targetNode = await migrateSnapshot(browser.id, migrationTarget, node)
      setMigrationNotice(`已迁移，目标节点 ${targetNode}`)
    } catch (err) {
      setSnapshotError(errorText(err))
    } finally {
      setSnapshotAction(null)
    }
  }

  const migrateToNew = async (node: string) => {
    if (!browser || !kind) return
    if (!window.confirm(`在本机按当前规格新建环境，并把快照 ${node} 迁移过去？`)) return
    setSnapshotAction(`new:${node}`)
    setSnapshotError(null)
    setMigrationNotice('')
    try {
      const result = await migrateSnapshotNew(browser.id, kind, kind, node)
      setMigrationNotice(
        `已新建 ${result.targetContainerId}，节点 ${result.targetNode}（${result.agentdAddr}）`,
      )
      await load()
    } catch (err) {
      setSnapshotError(errorText(err))
    } finally {
      setSnapshotAction(null)
    }
  }

  return (
    <section>
      <div className="section-head">
        <div>
          <h2>全部环境</h2>
          <p>连接环境后可以打开窗口、运行命令；运行软件支持创建、启动、停止和删除环境。</p>
        </div>
        <div className="row-actions">
          {canManage && (
            <button className="ghost" onClick={() => setCreating((value) => !value)}>
              {creating ? '取消创建' : '创建环境'}
            </button>
          )}
          <button className="ghost" onClick={() => void load()} disabled={loading || kind === null}>
            {loading ? '读取中…' : '刷新'}
          </button>
        </div>
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

          {creating && canManage && (
            <section className="connect-panel" aria-label="创建环境">
              <div className="section-head">
                <div>
                  <h3>创建 {kind === 'docker' ? 'Docker' : kind === 'wsl2' ? 'WSL 2' : 'Apple Container'} 环境</h3>
                  <p>
                    {kind === 'wsl2'
                      ? '从 rootfs tar 导入一个 mixumOS 节点；完成后在环境行连接 MAP。'
                      : '用本机镜像启动一个环境；完成后在环境行连接 MAP。'}
                  </p>
                </div>
              </div>
              <div className="connect-grid">
                <div className="field">
                  <label htmlFor="create-name">环境名称</label>
                  <input
                    id="create-name"
                    className="mono"
                    value={createDraft.name}
                    onChange={(event) => setCreateDraft({ ...createDraft, name: event.target.value })}
                    spellCheck={false}
                  />
                </div>
                <div className="field">
                  <label htmlFor="create-image">
                    {kind === 'wsl2' ? 'rootfs tar 路径' : '镜像名'}
                  </label>
                  <input
                    id="create-image"
                    className="mono"
                    value={createDraft.image}
                    onChange={(event) => setCreateDraft({ ...createDraft, image: event.target.value })}
                    placeholder={kind === 'wsl2' ? 'C:\mixum\rootfs.tar' : 'mixumos:0.1.4'}
                    spellCheck={false}
                  />
                </div>
                {kind !== 'wsl2' && (
                  <div className="field">
                    <label htmlFor="create-port">MAP 端口</label>
                    <input
                      id="create-port"
                      className="mono"
                      value={createDraft.hostPort}
                      onChange={(event) => setCreateDraft({ ...createDraft, hostPort: event.target.value })}
                      placeholder="17601"
                      spellCheck={false}
                    />
                  </div>
                )}
              </div>
              {error && <div className="error">{error}</div>}
              <div className="connect-actions">
                <button
                  className="primary"
                  onClick={() => void create()}
                  disabled={actionId === 'create'}
                >
                  {actionId === 'create' ? '导入中…' : '导入环境'}
                </button>
              </div>
            </section>
          )}

          {terminal && (
            <section className="connect-panel terminal-panel" aria-label={`${terminal.name} 的命令终端`}>
              <div className="section-head">
                <div>
                  <h3>{terminal.name} · 命令终端</h3>
                  <p>持久 PTY：直接通过运行时的 exec 进入容器；关闭面板即结束会话。</p>
                </div>
                <div className="row-actions">
                  <button className="ghost" onClick={() => void loadLogs()} disabled={terminalRunning}>
                    加载环境日志
                  </button>
                  <button className="ghost" onClick={() => void closeTerminal()}>
                    关闭
                  </button>
                </div>
              </div>
              <pre className="terminal-output">
                {terminalLines.length > 0 ? terminalLines.join('\n') : '输入命令并回车。'}
              </pre>
              {terminalError && <div className="error">{terminalError}</div>}
              <form
                className="terminal-input"
                onSubmit={(event) => {
                  event.preventDefault()
                  void runCommand()
                }}
              >
                <span className="mono">$</span>
                <input
                  className="mono"
                  value={terminalCommand}
                  onChange={(event) => setTerminalCommand(event.target.value)}
                  placeholder="例如 ls -la /var/lib/mixum"
                  spellCheck={false}
                  autoFocus
                />
                <button className="primary" type="submit" disabled={terminalRunning}>
                  {terminalRunning ? '执行中…' : '运行'}
                </button>
              </form>
            </section>
          )}

          {editor && (
            <section className="connect-panel" aria-label={`连接 ${editor.name}`}>
              <div className="section-head">
                <div>
                  <h3>连接 {editor.name}</h3>
                  <p>MART 可从容器内 admin-token 自动配对；令牌写入系统钥匙串，不会显示或写进快照。</p>
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
                  <label htmlFor="container-map-token">访问令牌（仅手动连接）</label>
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
                <button className="primary" onClick={() => void autoConnect()}>
                  自动连接
                </button>
                <button className="ghost" onClick={() => void saveAndBrowse()}>
                  手动保存令牌
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
                <div className="row-actions">
                  {surfaceError && !surfaces?.length && (
                    <button className="ghost" onClick={() => openManual(browser)}>
                      手动远程连接
                    </button>
                  )}
                  <button className="ghost" onClick={() => void openTerminal(browser)}>
                    终端
                  </button>
                  <button className="ghost" onClick={() => setBrowser(null)}>
                    关闭
                  </button>
                </div>
              </div>
              {surfaceError && <div className="error">{surfaceError}</div>}
              {surfaceLoading || surfaces === null ? (
                <p className="muted">正在查找窗口…</p>
              ) : surfaces.length === 0 ? (
                <p className="muted">这个环境里暂时没有窗口。</p>
              ) : (
                <div className="table-scroll"><table className="table">
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
                </table></div>
              )}

              <div className="snapshot-panel">
                <div className="section-head">
                  <div>
                    <h4>快照</h4>
                    <p className="muted">
                      {snapshots?.branch ? `分支 ${snapshots.branch}` : '读取当前快照树'}
                    </p>
                  </div>
                  <button
                    className="ghost"
                    onClick={() => void loadSnapshots(browser)}
                    disabled={snapshotLoading}
                  >
                    刷新
                  </button>
                </div>
                <div className="snapshot-create">
                  <select
                    value={snapshotKind}
                    onChange={(event) => setSnapshotKind(event.target.value)}
                    title="memory/hybrid 需要运行中的任务 + CRIU"
                  >
                    <option value="disk">磁盘</option>
                    <option value="memory">内存</option>
                    <option value="hybrid">混合</option>
                  </select>
                  <input
                    value={snapshotLabel}
                    onChange={(event) => setSnapshotLabel(event.target.value)}
                    placeholder="快照标签（可选）"
                    spellCheck={false}
                  />
                  <button
                    className="ghost"
                    onClick={() => void createSnapshot()}
                    disabled={snapshotAction === 'create'}
                  >
                    创建快照
                  </button>
                </div>
                <div className="snapshot-create">
                  <select
                    value={migrationPath}
                    onChange={(event) => setMigrationPath(event.target.value)}
                  >
                    <option value="">选择本机迁移包…</option>
                    {migrations.map((file) => (
                      <option key={file.path} value={file.path}>
                        {file.path.split('/').pop()}（{file.size} B）
                      </option>
                    ))}
                  </select>
                  <button className="ghost" onClick={() => void refreshMigrations()}>
                    扫描迁移包
                  </button>
                  <button
                    className="ghost"
                    onClick={() => void importMigration()}
                    disabled={!migrationPath || snapshotAction === 'import'}
                  >
                    导入
                  </button>
                </div>
                <div className="snapshot-create">
                  <select
                    value={migrationTarget}
                    onChange={(event) => setMigrationTarget(event.target.value)}
                  >
                    <option value="">迁移目标环境…</option>
                    {(containers ?? [])
                      .filter((container) => container.id !== browser.id)
                      .map((container) => (
                        <option key={container.id} value={container.id}>
                          {container.name}
                        </option>
                      ))}
                  </select>
                </div>
                {migrationNotice && <p className="muted">{migrationNotice}</p>}
                {snapshotError && <div className="error">{snapshotError}</div>}
                {snapshotLoading ? (
                  <p className="muted">读取快照树…</p>
                ) : !snapshots || (snapshots.nodes ?? []).length === 0 ? (
                  <p className="muted">暂无快照。</p>
                ) : (
                  <div className="table-scroll"><table className="table snapshot-table">
                    <thead>
                      <tr>
                        <th>快照</th>
                        <th>分支</th>
                        <th>类型</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {(snapshots.nodes ?? []).map((node) => (
                        <tr key={node.id}>
                          <td>
                            <div className="cell-stack">
                              <span>{node.label || node.id.slice(0, 12)}</span>
                              <span className="mono muted">{node.id.slice(0, 16)}</span>
                            </div>
                          </td>
                          <td className="mono muted">{node.branch}</td>
                          <td className="tag">{node.kind}</td>
                          <td className="row-actions">
                            <button
                              className="ghost"
                              onClick={() => void restoreSnapshot(node.id)}
                              disabled={snapshotAction === node.id}
                            >
                              回退
                            </button>
                            <button
                              className="ghost"
                              onClick={() => void exportSnapshot(node.id)}
                              disabled={snapshotAction === `export:${node.id}`}
                            >
                              导出
                            </button>
                            <button
                              className="ghost"
                              onClick={() => void migrateToTarget(node.id)}
                              disabled={!migrationTarget || snapshotAction === `migrate:${node.id}`}
                            >
                              迁移
                            </button>
                            <button
                              className="ghost"
                              onClick={() => void migrateToNew(node.id)}
                              disabled={snapshotAction === `new:${node.id}`}
                            >
                              新环境迁移
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table></div>
                )}
              </div>
            </section>
          )}

          {containers === null ? (
            <p className="muted">读取中…</p>
          ) : containers.length === 0 ? (
            <div className="empty-panel">
              <h3>这里还没有环境</h3>
              <p>点击上方「创建环境」开始。</p>
            </div>
          ) : (
            <div className="table-scroll"><table className="table">
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
                        {isMapImage(container.image) ? (
                          <button
                            className="ghost"
                            onClick={() => void openEnvironment(container)}
                            disabled={actionId === container.id}
                          >
                            {actionId === container.id
                              ? '连接中…'
                              : connection
                                ? '查看窗口'
                                : '打开'}
                          </button>
                        ) : (
                          <span className="muted" title="没有 mixumOS agentd/MAP">
                            无 MAP
                          </span>
                        )}
                        {canManage && container.state !== 'running' && container.state !== 'paused' && (
                          <button
                            className="ghost"
                            onClick={() => void runAction(container, 'start')}
                            disabled={actionId === container.id}
                          >
                            启动
                          </button>
                        )}
                        {canManage && container.state === 'running' && (
                          <button
                            className="ghost"
                            onClick={() => void runAction(container, 'stop')}
                            disabled={actionId === container.id}
                          >
                            停止
                          </button>
                        )}
                        <button
                          className="ghost"
                          onClick={() => void runAction(container, 'remove')}
                          disabled={actionId === container.id}
                        >
                          删除
                        </button>
                        <button
                          className="ghost"
                          onClick={() => void openTerminal(container)}
                          title="进入容器持久终端（本地 exec）"
                        >
                          终端
                        </button>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table></div>
          )}
        </>
      )}
    </section>
  )
}
