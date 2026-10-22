import { useEffect, useRef, useState } from 'react'

import {
  connectEnvironment,
  errorText,
  gatewayDevices,
  gatewayIsGateway,
  gatewayNodes,
  getAgentCredential,
  gatewayPairClaim,
  gatewayPairCreate,
  gatewayRevokeDevice,
  listContainers,
  listEnvironmentSurfaces,
  listSurfaces,
  openEnvironmentSurface,
  openSurfaceWindow,
  setAgentCredential,
  startEnvironment,
} from '../api'
import type { DeviceInfo, PairClaim } from '../api'
import type { BackendStatus, Container, RuntimeKind, SurfaceInfo } from '../types'

const DEFAULT_ADDR = '127.0.0.1:7601'

/** A = mixcomp 兼容通道，B = 合成器增强通道。对用户只展示「增强 / 兼容」。 */
const isEnhanced = (surface: SurfaceInfo) => surface.id >= 1000

const isMapImage = (image: string) => image.toLowerCase().includes('mixumos')

const defaultAddress = (container: Container) => {
  const port =
    container.ports.find((item) => item.container === 7601) ?? container.ports[0]
  return port ? `127.0.0.1:${port.host}` : DEFAULT_ADDR
}

type Group = {
  kind: RuntimeKind
  container: Container
  surfaces: SurfaceInfo[] | null
  error: string | null
  busy: boolean
}

type Props = {
  backends?: BackendStatus[] | null
  onOpenEnvironments?: () => void
}

export function Surfaces({ backends, onOpenEnvironments }: Props) {
  const [groups, setGroups] = useState<Group[]>([])
  const [discovering, setDiscovering] = useState(false)
  const [manual, setManual] = useState(false)
  const [addr, setAddr] = useState(
    () => localStorage.getItem('mixum.map.addr') ?? DEFAULT_ADDR,
  )
  const [token, setToken] = useState('')
  const [deviceToken, setDeviceToken] = useState('')
  const [fingerprint, setFingerprint] = useState(
    () => localStorage.getItem('mixum.map.fp') ?? '',
  )
  const [manualSurfaces, setManualSurfaces] = useState<SurfaceInfo[] | null>(null)
  const [node, setNode] = useState(() => localStorage.getItem('mixum.map.node') ?? '')
  const [nodes, setNodes] = useState<string[]>([])
  const [pairCode, setPairCode] = useState('')
  const [claimCode, setClaimCode] = useState('')
  const [claimName, setClaimName] = useState('MixumART')
  const [claimResult, setClaimResult] = useState<PairClaim | null>(null)
  const [devices, setDevices] = useState<DeviceInfo[]>([])
  const [route, setRoute] = useState<'all' | 'enhanced' | 'compat'>('all')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const patchGroup = (containerId: string, patch: Partial<Group>) => {
    setGroups((previous) =>
      previous.map((group) =>
        group.container.id === containerId ? { ...group, ...patch } : group,
      ),
    )
  }

  const connectGroup = async (group: Group, start: boolean) => {
    patchGroup(group.container.id, { busy: true, error: null })
    try {
      if (start && group.container.state !== 'running') {
        await startEnvironment(group.kind, group.container.id)
        await new Promise((resolve) => setTimeout(resolve, 700))
      }
      await connectEnvironment(group.kind, group.container.id, defaultAddress(group.container))
      const surfaces = await listEnvironmentSurfaces(group.container.id)
      patchGroup(group.container.id, { surfaces, error: null, busy: false })
    } catch (err) {
      patchGroup(group.container.id, {
        surfaces: [],
        busy: false,
        error: `${errorText(err)}。只有带 mixumOS agentd 的容器能提供 MAP 窗口。`,
      })
    }
  }

  const discover = async () => {
    setDiscovering(true)
    setError(null)
    try {
      const ready = (backends ?? []).filter((backend) => backend.status.state === 'ready')
      const discovered: Group[] = []
      for (const backend of ready) {
        const containers = await listContainers(backend.kind)
        for (const container of containers) {
          if (!isMapImage(container.image)) continue
          discovered.push({
            kind: backend.kind,
            container,
            surfaces: null,
            error: null,
            busy: false,
          })
        }
      }
      setGroups(discovered)
      // 运行中的环境直接自动配对并列出窗口；停止的只展示“启动并查看”。
      for (const group of discovered) {
        if (group.container.state === 'running') await connectGroup(group, false)
      }
    } catch (err) {
      setError(errorText(err))
    } finally {
      setDiscovering(false)
    }
  }

  const prevAddr = useRef(addr)

  useEffect(() => {
    let cancelled = false
    // 切换 gateway 地址时必须清掉上一个地址的 bearer token,避免把 A 的
    // 用户/设备令牌发给 B;节点选择也一并重置。
    if (prevAddr.current !== addr) {
      prevAddr.current = addr
      setToken('')
      setDeviceToken('')
      setNodes([])
      setNode('')
      localStorage.removeItem('mixum.map.node')
    }
    void getAgentCredential(`gateway-device:${addr}`)
      .then((saved) => {
        if (!cancelled && saved) setDeviceToken(saved)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [addr])

  useEffect(() => {
    localStorage.removeItem('mixum.map.token')
    void discover()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backends])

  const scanManual = async () => {
    const scanToken = token.trim() || deviceToken.trim()
    if (!scanToken) {
      setError('手动连接远程节点需要访问令牌；本机环境会自动配对，不需要填写。')
      return
    }
    localStorage.setItem('mixum.map.addr', addr)
    localStorage.setItem('mixum.map.fp', fingerprint)
    localStorage.setItem('mixum.map.node', node)
    setLoading(true)
    setError(null)
    try {
      // 先取节点列表,再按选中的节点列窗口;否则多节点 gateway 在 node 为空时
      // 会先失败,下拉框永远没有机会出现。
      let list: string[] = []
      // 直连 agentd 没有 gateway.node.list:先探测,避免把“不是 gateway”误报成错误。
      const isGateway = await gatewayIsGateway(addr, scanToken, fingerprint || undefined)
      if (isGateway) {
        list = await gatewayNodes(addr, scanToken, fingerprint || undefined)
        setNodes(list)
      } else {
        setNodes([])
      }
      const effective = node && list.includes(node) ? node : (list[0] ?? '')
      if (effective !== node) setNode(effective)
      setManualSurfaces(
        await listSurfaces(
          addr,
          deviceToken || token,
          fingerprint || undefined,
          effective || undefined,
        ),
      )
    } catch (err) {
      setError(errorText(err))
      setManualSurfaces([])
    } finally {
      setLoading(false)
    }
  }

  const shown = (surfaces: SurfaceInfo[]) =>
    surfaces.filter((surface) => {
      if (route === 'all') return true
      return route === 'enhanced' ? isEnhanced(surface) : !isEnhanced(surface)
    })

  const open = async (group: Group, surface: SurfaceInfo) => {
    setError(null)
    try {
      await openEnvironmentSurface(group.container.id, surface)
    } catch (err) {
      setError(errorText(err))
    }
  }

  const openManual = async (surface: SurfaceInfo) => {
    setError(null)
    try {
      await openSurfaceWindow(
        addr,
        deviceToken || token,
        fingerprint || undefined,
        surface,
        node || undefined,
      )
    } catch (err) {
      setError(errorText(err))
    }
  }

  const createPair = async () => {
    setError(null)
    try {
      const pair = await gatewayPairCreate(addr, token, fingerprint || undefined)
      setPairCode(`${pair.code}（${Math.round(pair.expiresIn / 60)} 分钟内有效）`)
    } catch (err) {
      setError(errorText(err))
    }
  }

  const claimDevice = async () => {
    setError(null)
    try {
      const claim = await gatewayPairClaim(
        addr,
        claimCode.trim(),
        claimName.trim() || 'MixumART',
        fingerprint || undefined,
      )
      setClaimResult(claim)
      setClaimCode('')
      setDeviceToken(claim.token)
      try {
        await setAgentCredential(`gateway-device:${addr}`, claim.token)
      } catch (err) {
        setError(`设备已配对，但写入系统钥匙串失败：${errorText(err)}`)
      }
    } catch (err) {
      setError(errorText(err))
    }
  }

  const loadDevices = async () => {
    setError(null)
    try {
      setDevices(await gatewayDevices(addr, token, fingerprint || undefined))
    } catch (err) {
      setError(errorText(err))
    }
  }

  const revokeDevice = async (jti: string) => {
    setError(null)
    try {
      await gatewayRevokeDevice(addr, token, fingerprint || undefined, jti)
      await loadDevices()
    } catch (err) {
      setError(errorText(err))
    }
  }

  const connectWithDevice = async () => {
    if (!deviceToken.trim()) {
      setError('没有设备令牌:请先用配对码接入设备。')
      return
    }
    setLoading(true)
    setError(null)
    try {
      // 设备令牌同样可以 node.list(RFC 0003);多节点时先选节点再列窗口。
      const list = await gatewayNodes(addr, deviceToken, fingerprint || undefined)
      setNodes(list)
      const effective = node && list.includes(node) ? node : (list[0] ?? '')
      if (effective !== node) setNode(effective)
      setManualSurfaces(
        await listSurfaces(
          addr,
          deviceToken,
          fingerprint || undefined,
          effective || undefined,
        ),
      )
    } catch (err) {
      setError(errorText(err))
      setManualSurfaces([])
    } finally {
      setLoading(false)
    }
  }

  const totalWindows = groups.reduce((total, group) => total + (group.surfaces?.length ?? 0), 0)

  return (
    <section>
      <div className="section-head">
        <div>
          <h2>环境里的窗口</h2>
          <p>
            自动发现带 mixumOS agentd 的容器；已运行的环境直接列出窗口，不需要填写令牌。
          </p>
        </div>
        <div className="row-actions">
          <button className="ghost" onClick={() => void discover()} disabled={discovering}>
            {discovering ? '发现中…' : '刷新环境'}
          </button>
          <button className="ghost" onClick={() => setManual((value) => !value)}>
            {manual ? '收起远程连接' : '远程节点'}
          </button>
        </div>
      </div>

      {error && <div className="error">{error}</div>}

      {manual && (
        <div className="chips" style={{ alignItems: 'flex-end' }}>
          <div className="field">
            <label htmlFor="map-addr">远程节点地址</label>
            <input
              id="map-addr"
              className="mono"
              value={addr}
              onChange={(event) => setAddr(event.target.value)}
              placeholder="例如 127.0.0.1:7601"
              spellCheck={false}
            />
          </div>
          <div className="field">
            <label htmlFor="map-token">访问令牌</label>
            <input
              id="map-token"
              className="mono"
              type="password"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              placeholder="map1.…"
              spellCheck={false}
            />
          </div>
          <div className="field">
            <label htmlFor="map-fp">证书指纹（可选）</label>
            <input
              id="map-fp"
              className="mono"
              value={fingerprint}
              onChange={(event) => setFingerprint(event.target.value)}
              placeholder="首次连接可留空"
              spellCheck={false}
            />
          </div>
          {nodes.length > 0 && (
            <div className="field">
              <label htmlFor="map-node">节点（gateway）</label>
              <select
                id="map-node"
                value={node}
                onChange={(event) => {
                  const next = event.target.value
                  setNode(next)
                  localStorage.setItem('mixum.map.node', next)
                  setManualSurfaces(null)
                  if (!token.trim()) return
                  setLoading(true)
                  setError(null)
                  void listSurfaces(
                  addr,
                  deviceToken || token,
                  fingerprint || undefined,
                  next || undefined,
                )
                    .then(setManualSurfaces)
                    .catch((err) => setError(errorText(err)))
                    .finally(() => setLoading(false))
                }}
              >
                <option value="">自动</option>
                {nodes.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </div>
          )}
          <button className="ghost" onClick={() => void scanManual()} disabled={loading}>
            {loading ? '查找中…' : '查找远程窗口'}
          </button>
        </div>
      )}

      <details className="gateway-panel">
        <summary>Gateway 配对与设备</summary>
        <div className="chips" style={{ alignItems: 'flex-end' }}>
          <div className="field">
            <label>配对码</label>
            <input className="mono" value={pairCode} readOnly placeholder="点击生成" />
          </div>
          <button className="ghost" onClick={() => void createPair()} disabled={!token.trim()}>
            生成配对码
          </button>
        </div>
        <div className="chips" style={{ alignItems: 'flex-end' }}>
          <div className="field">
            <label htmlFor="claim-code">设备配对码</label>
            <input
              id="claim-code"
              className="mono"
              maxLength={6}
              value={claimCode}
              onChange={(event) => setClaimCode(event.target.value)}
              placeholder="6 位数字"
            />
          </div>
          <div className="field">
            <label htmlFor="claim-name">设备名</label>
            <input
              id="claim-name"
              value={claimName}
              onChange={(event) => setClaimName(event.target.value)}
            />
          </div>
          <button className="ghost" onClick={() => void claimDevice()} disabled={!claimCode.trim()}>
            配对
          </button>
        </div>
        {claimResult && (
          <>
            <p className="mode-note">
              已配对设备 <span className="mono">{claimResult.deviceId}</span>
              （设备令牌已写入系统钥匙串并填入设备令牌）。
            </p>
          </>
        )}
        <div className="chips">
          <button className="ghost" onClick={() => void loadDevices()} disabled={!token.trim()}>
            刷新设备
          </button>
        </div>
        {devices.length > 0 && (
          <table className="table">
            <thead>
              <tr>
                <th>设备</th>
                <th>主体</th>
                <th>属主</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {devices.map((device) => (
                <tr key={device.jti}>
                  <td>{device.name || device.jti}</td>
                  <td className="mono muted">{device.sub}</td>
                  <td className="mono muted">{device.owner}</td>
                  <td className="row-actions">
                    <button className="ghost" onClick={() => void revokeDevice(device.jti)}>
                      吊销
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="mode-note">配对码 5 分钟有效；设备令牌独立于当前用户令牌，可单独吊销。</p>
      </details>
        {deviceToken.trim() && (
          <button className="ghost" onClick={() => void connectWithDevice()} disabled={loading}>
            用设备令牌连接
          </button>
        )}


      <p className="mode-note">
        <strong>增强模式</strong>画面更清晰、流量更省，推荐使用。
        <strong>兼容模式</strong>用于 X11 或少数显示异常的应用，作为备用。
      </p>

      <div className="chips">
        {(
          [
            ['all', '全部'],
            ['enhanced', '增强模式'],
            ['compat', '兼容模式'],
          ] as const
        ).map(([value, label]) => (
          <button
            key={value}
            className={route === value ? 'chip active' : 'chip'}
            onClick={() => setRoute(value)}
          >
            {label}
          </button>
        ))}
      </div>

      {manual && manualSurfaces && (
        <EnvironmentWindows
          title="远程节点"
          subtitle={addr}
          surfaces={shown(manualSurfaces)}
          empty="这个远程环境里暂时没有窗口。"
          onOpen={openManual}
        />
      )}

      {!manual && groups.length === 0 && !discovering && (
        <div className="empty">
          没有发现 mixumOS 环境。普通容器没有 MAP agentd，只能在环境页打开终端。
        </div>
      )}

      {!manual &&
        groups.map((group) => (
          <EnvironmentWindows
            key={`${group.kind}:${group.container.id}`}
            title={group.container.name}
            subtitle={`${group.container.image} · ${group.container.state}`}
            surfaces={shown(group.surfaces ?? [])}
            empty={
              group.busy
                ? '正在连接并读取窗口…'
                : group.container.state !== 'running'
                  ? '容器已停止；启动后会自动读取窗口。'
                  : '这个环境里暂时没有窗口。'
            }
            error={group.error}
            action={
              group.container.state !== 'running' ? (
                <button
                  className="ghost"
                  onClick={() => void connectGroup(group, true)}
                  disabled={group.busy}
                >
                  {group.busy ? '启动中…' : '启动并查看'}
                </button>
              ) : group.surfaces === null ? (
                <button className="ghost" onClick={() => void connectGroup(group, false)}>
                  重试连接
                </button>
              ) : (
                <button
                  className="ghost"
                  onClick={() => onOpenEnvironments?.()}
                >
                  终端 / 管理
                </button>
              )
            }
            onOpen={(surface) => void open(group, surface)}
          />
        ))}

      {totalWindows === 0 && !manual && groups.length > 0 && (
        <p className="muted">当前没有可显示的窗口；停止的环境可以先启动。</p>
      )}
    </section>
  )
}

type WindowsProps = {
  title: string
  subtitle: string
  surfaces: SurfaceInfo[]
  empty: string
  error?: string | null
  action?: React.ReactNode
  onOpen: (surface: SurfaceInfo) => void
}

function EnvironmentWindows({
  title,
  subtitle,
  surfaces,
  empty,
  error,
  action,
  onOpen,
}: WindowsProps) {
  return (
    <article className="card" style={{ marginTop: 14 }}>
      <div className="card-head">
        <div>
          <h3>{title}</h3>
          <p className="card-note mono">{subtitle}</p>
        </div>
        {action}
      </div>
      {error && <div className="error">{error}</div>}
      {surfaces.length === 0 ? (
        <p className="muted">{empty}</p>
      ) : (
        <div className="table-scroll"><table className="table">
          <thead>
            <tr>
              <th>方式</th>
              <th>窗口</th>
              <th>应用</th>
              <th>尺寸</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {surfaces.map((surface) => (
              <tr key={surface.id}>
                <td>
                  <span className={isEnhanced(surface) ? 'tag tag-a' : 'tag'}>
                    {isEnhanced(surface) ? '增强' : '兼容'}
                  </span>
                </td>
                <td>{surface.title || '未命名窗口'}</td>
                <td className="mono muted">{surface.appId || '—'}</td>
                <td className="mono">
                  {surface.width}×{surface.height}
                </td>
                <td className="row-actions">
                  <button className="ghost" onClick={() => onOpen(surface)}>
                    打开窗口
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table></div>
      )}
    </article>
  )
}
