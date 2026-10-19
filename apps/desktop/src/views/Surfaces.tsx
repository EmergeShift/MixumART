import { useState } from 'react'

import { errorText, listSurfaces, openSurfaceWindow } from '../api'
import type { SurfaceInfo } from '../types'

const DEFAULT_ADDR = '127.0.0.1:7601'

export function Surfaces() {
  const [addr, setAddr] = useState(
    () => localStorage.getItem('mixum.map.addr') ?? DEFAULT_ADDR,
  )
  const [token, setToken] = useState(() => localStorage.getItem('mixum.map.token') ?? '')
  const [fingerprint, setFingerprint] = useState(
    () => localStorage.getItem('mixum.map.fp') ?? '',
  )
  const [surfaces, setSurfaces] = useState<SurfaceInfo[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const scan = async () => {
    if (!token.trim()) {
      setError('需要 MAP 令牌（map1.…）')
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
          <h2>窗口</h2>
          <p className="muted">连接 art compositor，每个 surface 打开一个独立窗口。</p>
        </div>
        <button className="ghost" onClick={() => void scan()} disabled={loading}>
          {loading ? '扫描中…' : '扫描 surface'}
        </button>
      </div>
      <div className="chips">
        <input
          className="mono surface-addr"
          value={addr}
          onChange={(event) => setAddr(event.target.value)}
          placeholder="MAP 地址，例如 127.0.0.1:7601"
          spellCheck={false}
        />
        <input
          className="mono surface-token"
          type="password"
          value={token}
          onChange={(event) => setToken(event.target.value)}
          placeholder="MAP 令牌 map1.…"
          spellCheck={false}
        />
        <input
          className="mono surface-fp"
          value={fingerprint}
          onChange={(event) => setFingerprint(event.target.value)}
          placeholder="证书指纹（可选，留空 TOFU）"
          spellCheck={false}
        />
      </div>
      {error && <div className="error">{error}</div>}
      {surfaces === null ? (
        <div className="empty">输入 art compositor 地址后扫描。</div>
      ) : surfaces.length === 0 ? (
        <div className="empty">没有发现 surface。</div>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>ID</th>
              <th>标题</th>
              <th>应用</th>
              <th>尺寸</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {surfaces.map((surface) => (
              <tr key={surface.id}>
                <td className="mono">{surface.id}</td>
                <td>{surface.title || '—'}</td>
                <td className="mono">{surface.appId || '—'}</td>
                <td className="mono">
                  {surface.width}×{surface.height}
                </td>
                <td>
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
