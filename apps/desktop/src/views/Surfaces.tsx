import { useEffect, useState } from 'react'

import {
  errorText,
  listEnvironmentSurfaces,
  listSurfaces,
  loadEnvironmentConnections,
  openEnvironmentSurface,
  openSurfaceWindow,
} from '../api'
import type { ConnectionInfo, SurfaceInfo } from '../types'

const DEFAULT_ADDR = '127.0.0.1:7601'

/** A = mixcomp 兼容通道，B = 合成器增强通道。对用户只展示「增强 / 兼容」。 */
const isEnhanced = (surface: SurfaceInfo) => surface.id >= 1000

export function Surfaces() {
  const [connections, setConnections] = useState<Record<string, ConnectionInfo>>({})
  const [selected, setSelected] = useState('')
  const [manual, setManual] = useState(false)
  const [addr, setAddr] = useState(
    () => localStorage.getItem('mixum.map.addr') ?? DEFAULT_ADDR,
  )
  const [token, setToken] = useState('')
  const [fingerprint, setFingerprint] = useState(
    () => localStorage.getItem('mixum.map.fp') ?? '',
  )
  const [surfaces, setSurfaces] = useState<SurfaceInfo[] | null>(null)
  const [route, setRoute] = useState<'all' | 'enhanced' | 'compat'>('all')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refreshConnections = async () => {
    try {
      const next = await loadEnvironmentConnections()
      setConnections(next)
      setSelected((previous) => previous || Object.keys(next)[0] || '')
    } catch (err) {
      setError(errorText(err))
    }
  }

  useEffect(() => {
    // 旧版本曾把 MAP token 存在 localStorage；新流程只用钥匙串。
    localStorage.removeItem('mixum.map.token')
    void refreshConnections()
  }, [])

  const useManual = manual || Object.keys(connections).length === 0
  const scan = async () => {
    setLoading(true)
    setError(null)
    try {
      if (useManual) {
        if (!token.trim()) {
          setError('手动连接远程节点需要访问令牌；本机环境请在环境页自动连接。')
          return
        }
        localStorage.setItem('mixum.map.addr', addr)
        localStorage.setItem('mixum.map.fp', fingerprint)
        setSurfaces(await listSurfaces(addr, token, fingerprint || undefined))
      } else {
        if (!selected) {
          setError('请选择一个已连接的环境')
          return
        }
        setSurfaces(await listEnvironmentSurfaces(selected))
      }
    } catch (err) {
      setError(errorText(err))
      setSurfaces([])
    } finally {
      setLoading(false)
    }
  }

  const shown = (surfaces ?? []).filter((surface) => {
    if (route === 'all') return true
    return route === 'enhanced' ? isEnhanced(surface) : !isEnhanced(surface)
  })

  const open = async (surface: SurfaceInfo) => {
    setError(null)
    try {
      if (useManual) {
        await openSurfaceWindow(addr, token, fingerprint || undefined, surface)
      } else {
        await openEnvironmentSurface(selected, surface)
      }
    } catch (err) {
      setError(errorText(err))
    }
  }

  return (
    <section>
      <div className="section-head">
        <div>
          <h2>环境里的窗口</h2>
          <p>
            已连接的环境不需要再填令牌；MART 通过本机钥匙串里的配对信息连接。
            远程节点可以切到手动模式。
          </p>
        </div>
        <div className="row-actions">
          <button className="ghost" onClick={() => void refreshConnections()}>
            刷新环境
          </button>
          <button className="ghost" onClick={() => void scan()} disabled={loading}>
            {loading ? '查找中…' : '查找窗口'}
          </button>
        </div>
      </div>

      <div className="chips" style={{ alignItems: 'flex-end' }}>
        {!useManual ? (
          <div className="field">
            <label htmlFor="map-environment">已连接环境</label>
            <select
              id="map-environment"
              value={selected}
              onChange={(event) => {
                setSelected(event.target.value)
                setSurfaces(null)
              }}
            >
              {Object.entries(connections).map(([id, connection]) => (
                <option key={id} value={id}>
                  {connection.addr}
                </option>
              ))}
            </select>
          </div>
        ) : (
          <>
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
          </>
        )}
        {Object.keys(connections).length > 0 && (
          <button className="ghost" onClick={() => setManual((value) => !value)}>
            {manual ? '使用已连接环境' : '手动连接远程节点'}
          </button>
        )}
      </div>

      {Object.keys(connections).length === 0 && (
        <p className="mode-note">
          还没有已连接环境。请在「环境」页连接一个节点；本机环境会自动读取容器内 token，
          不需要用户手动输入。
        </p>
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

      {error && <div className="error">{error}</div>}

      {surfaces === null ? (
        <div className="empty">
          {useManual
            ? '填入远程地址与访问令牌后，点击「查找窗口」。'
            : '选择一个已连接环境，点击「查找窗口」。'}
        </div>
      ) : shown.length === 0 ? (
        <div className="empty">
          {surfaces.length === 0 ? '这个环境里暂时没有窗口。' : '当前筛选方式下没有窗口。'}
        </div>
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
            {shown.map((surface) => (
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
                  <button className="ghost" onClick={() => void open(surface)}>
                    打开窗口
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table></div>
      )}
    </section>
  )
}
