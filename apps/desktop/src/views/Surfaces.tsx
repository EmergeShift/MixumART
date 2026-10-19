import { useState } from 'react'

import { errorText, listSurfaces, openSurfaceWindow } from '../api'
import type { SurfaceInfo } from '../types'

const DEFAULT_ADDR = '127.0.0.1:7603'

export function Surfaces() {
  const [addr, setAddr] = useState(
    () => localStorage.getItem('mixum.art.addr') ?? DEFAULT_ADDR,
  )
  const [surfaces, setSurfaces] = useState<SurfaceInfo[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const scan = async () => {
    localStorage.setItem('mixum.art.addr', addr)
    setLoading(true)
    setError(null)
    try {
      setSurfaces(await listSurfaces(addr))
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
      await openSurfaceWindow(addr, surface)
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
          placeholder="art compositor 地址，例如 192.168.64.32:7603"
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
