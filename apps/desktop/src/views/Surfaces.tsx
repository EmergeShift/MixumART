import { useState } from 'react'

import { errorText, listSurfaces, openSurfaceWindow } from '../api'
import type { SurfaceInfo } from '../types'

const DEFAULT_ADDR = '127.0.0.1:7601'

/** A = mixcomp 兼容通道，B = 合成器增强通道。对用户只展示「增强 / 兼容」。 */
const isEnhanced = (surface: SurfaceInfo) => surface.id >= 1000

export function Surfaces() {
  const [addr, setAddr] = useState(
    () => localStorage.getItem('mixum.map.addr') ?? DEFAULT_ADDR,
  )
  const [token, setToken] = useState(() => localStorage.getItem('mixum.map.token') ?? '')
  const [fingerprint, setFingerprint] = useState(
    () => localStorage.getItem('mixum.map.fp') ?? '',
  )
  const [surfaces, setSurfaces] = useState<SurfaceInfo[] | null>(null)
  const [route, setRoute] = useState<'all' | 'enhanced' | 'compat'>('all')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const scan = async () => {
    if (!token.trim()) {
      setError('请先填入访问令牌（map1.…）')
      return
    }
    localStorage.setItem('mixum.map.addr', addr)
    localStorage.setItem('mixum.map.token', token)
    localStorage.setItem('mixum.map.fp', fingerprint)
    setLoading(true)
    setError(null)
    try {
      setSurfaces(await listSurfaces(addr, token, fingerprint || undefined))
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
      await openSurfaceWindow(addr, token, fingerprint || undefined, surface)
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
            连接到一台环境后，可以在这里看到它正在运行的应用窗口，并打开独立窗口监看或接手。
          </p>
        </div>
        <button className="ghost" onClick={() => void scan()} disabled={loading}>
          {loading ? '查找中…' : '查找窗口'}
        </button>
      </div>

      <div className="chips" style={{ alignItems: 'flex-end' }}>
        <div className="field">
          <label htmlFor="map-addr">节点地址</label>
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
      </div>

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
        <div className="empty">填入节点地址与访问令牌后，点击「查找窗口」。</div>
      ) : shown.length === 0 ? (
        <div className="empty">
          {surfaces.length === 0 ? '这个环境里暂时没有窗口。' : '当前筛选方式下没有窗口。'}
        </div>
      ) : (
        <table className="table">
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
        </table>
      )}
    </section>
  )
}
