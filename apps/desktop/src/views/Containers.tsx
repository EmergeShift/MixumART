import { useCallback, useEffect, useMemo, useState } from 'react'

import { errorText, listContainers } from '../api'
import { statusDetail } from '../status'
import type { BackendStatus, Container, ContainerState, PortMapping, RuntimeKind } from '../types'

type Props = {
  backends: BackendStatus[] | null
}

export function Containers({ backends }: Props) {
  const ready = useMemo(
    () => (backends ?? []).filter((backend) => backend.status.state === 'ready'),
    [backends],
  )
  const [kind, setKind] = useState<RuntimeKind | null>(null)
  const [containers, setContainers] = useState<Container[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  // 默认选中第一个可用运行时。
  useEffect(() => {
    if (kind === null && ready.length > 0) setKind(ready[0].kind)
  }, [kind, ready])

  const load = useCallback(async () => {
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
  }, [kind])

  useEffect(() => {
    void load()
  }, [load])

  return (
    <section>
      <div className="section-head">
        <div>
          <h2>容器</h2>
          <p className="muted">各运行时里的 mixumOS 容器。骨架阶段只读，启停与创建在 M1 接入。</p>
        </div>
        <button className="ghost" onClick={() => void load()} disabled={loading || kind === null}>
          {loading ? '读取中…' : '刷新'}
        </button>
      </div>

      {backends === null ? (
        <p className="muted">正在检测运行时…</p>
      ) : (
        <>
          <div className="chips">
            {backends.map((backend) => (
              <button
                key={backend.kind}
                className={`chip ${kind === backend.kind ? 'active' : ''}`}
                disabled={backend.status.state !== 'ready'}
                title={statusDetail(backend.status)}
                onClick={() => {
                  setKind(backend.kind)
                  setContainers(null)
                }}
              >
                {backend.name}
              </button>
            ))}
          </div>

          {error && <div className="error">{error}</div>}

          {kind === null ? (
            <p className="empty">没有可用的运行时。先到「环境」里检测并启动一个。</p>
          ) : containers === null ? (
            <p className="muted">读取容器列表…</p>
          ) : containers.length === 0 ? (
            <p className="empty">该运行时下还没有容器。</p>
          ) : (
            <ContainerTable containers={containers} />
          )}
        </>
      )}
    </section>
  )
}

function ContainerTable({ containers }: { containers: Container[] }) {
  return (
    <table className="table">
      <thead>
        <tr>
          <th>名称</th>
          <th>镜像</th>
          <th>状态</th>
          <th>端口</th>
          <th>创建时间</th>
        </tr>
      </thead>
      <tbody>
        {containers.map((container) => (
          <tr key={container.id}>
            <td>
              <div className="name">{container.name}</div>
              <div className="mono">{shortId(container.id)}</div>
            </td>
            <td className="mono">{container.image}</td>
            <td>
              <span className={`state ${container.state}`}>{stateLabel(container.state)}</span>
              {container.status ? <div className="mono">{container.status}</div> : null}
            </td>
            <td className="mono">{formatPorts(container.ports)}</td>
            <td className="mono">{formatTime(container.createdAt)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

const STATE_LABELS: Record<ContainerState, string> = {
  created: '已创建',
  running: '运行中',
  paused: '已暂停',
  stopped: '已停止',
  unknown: '未知',
}

function stateLabel(state: ContainerState): string {
  return STATE_LABELS[state]
}

function shortId(id: string): string {
  return id.length > 12 ? id.slice(0, 12) : id
}

function formatPorts(ports: PortMapping[]): string {
  if (ports.length === 0) return '—'
  return ports.map((port) => `${port.host}→${port.container}`).join('  ')
}

function formatTime(value: string | null): string {
  if (!value) return '—'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString('zh-CN', { hour12: false })
}
