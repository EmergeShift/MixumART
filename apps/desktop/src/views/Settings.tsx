import { useEffect, useState } from 'react'

import { errorText, runtimeDiskUsage, runtimeImages, runtimeLoadImage, runtimePrune, runtimePullImage } from '../api'
import { statusDetail } from '../status'
import { StatusBadge } from '../components/StatusBadge'
import type { BackendStatus, HostInfo, RuntimeKind } from '../types'

type Props = {
  backends: BackendStatus[] | null
  host: HostInfo | null
  appearance: string
  onRefresh: () => void
  onSetup: () => void
}

export function Settings({ backends, host, appearance, onRefresh, onSetup }: Props) {
  const [resources, setResources] = useState<
    Record<string, { images: string[]; disk: string; error: string | null }>
  >({})
  const [refreshTick, setRefreshTick] = useState(0)
  const [pruning, setPruning] = useState<string | null>(null)
  const [pulling, setPulling] = useState<string | null>(null)
  const [imageDrafts, setImageDrafts] = useState<Record<string, string>>({})
  const [tarDrafts, setTarDrafts] = useState<Record<string, string>>({})
  const [loadingImage, setLoadingImage] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const ready = (backends ?? []).filter((backend) => backend.status.state === 'ready')
    void Promise.all(
      ready.map(async (backend) => {
        try {
          const [images, disk] = await Promise.all([
            runtimeImages(backend.kind),
            runtimeDiskUsage(backend.kind),
          ])
          return [backend.kind, { images, disk, error: null }] as const
        } catch (error: unknown) {
          return [backend.kind, { images: [], disk: '', error: String(error) }] as const
        }
      }),
    ).then((entries) => {
      if (!cancelled) setResources(Object.fromEntries(entries))
    })
  return () => {
      cancelled = true
    }
  }, [backends, refreshTick])

  const pull = async (kind: RuntimeKind) => {
    const reference = (imageDrafts[kind] ?? '').trim()
    if (!reference) return
    setPulling(kind)
    try {
      await runtimePullImage(kind, reference)
      setImageDrafts((previous) => ({ ...previous, [kind]: '' }))
      setRefreshTick((value) => value + 1)
    } catch (error) {
      window.alert(errorText(error))
    } finally {
      setPulling(null)
    }
  }

  const loadImage = async (kind: RuntimeKind) => {
    const path = (tarDrafts[kind] ?? '').trim()
    if (!path) return
    setLoadingImage(kind)
    try {
      await runtimeLoadImage(kind, path)
      setTarDrafts((previous) => ({ ...previous, [kind]: '' }))
      setRefreshTick((value) => value + 1)
    } catch (error) {
      window.alert(errorText(error))
    } finally {
      setLoadingImage(null)
    }
  }

  const prune = async (kind: RuntimeKind) => {
    if (!window.confirm(`清理 ${kind} 未使用的容器/镜像？这会删除未使用的数据。`)) return
    setPruning(kind)
    try {
      await runtimePrune(kind)
      setRefreshTick((value) => value + 1)
    } catch (error) {
      window.alert(errorText(error))
    } finally {
      setPruning(null)
    }
  }

  return (
    <div className="settings-page">
      <section>
        <div className="section-head">
          <div>
            <h2>运行软件</h2>
            <p>这台电脑上用来承载 Agent 环境的软件。装好其中一个即可。</p>
          </div>
          <button className="ghost" onClick={onRefresh}>
            重新检测
          </button>
        </div>
        {backends === null ? (
          <p className="muted">正在检测…</p>
        ) : (
          <div className="cards">
            {backends.map((backend) => (
              <article className="card" key={backend.kind}>
                <div className="card-head">
                  <h3>{backend.name}</h3>
                  <StatusBadge status={backend.status} />
                </div>
                <p className="card-note">{statusDetail(backend.status)}</p>
                <code className="mono">{backend.kind}</code>
              </article>
            ))}
          </div>
        )}
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>环境系统</h2>
            <p>每个环境里安装的系统是 mixumOS，这里是它的设置。</p>
          </div>
        </div>
        <div className="settings-list">
          <div className="setting-row">
            <div>
              <strong>快照与回退</strong>
              <span className="muted">自动保存工作进度，可以随时回到之前的状态</span>
            </div>
            <span className="mono muted">在环境页</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>内存快照</strong>
              <span className="muted">保留正在运行的进程；目前要求来源和目标使用同一内核</span>
            </div>
            <span className="mono muted">同一内核</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>系统镜像</strong>
              <span className="muted">在环境页创建时选择；下方列出运行软件当前可见的镜像</span>
            </div>
            <span className="mono muted">见下方</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>磁盘占用</strong>
              <span className="muted">运行软件的镜像、容器与卷占用</span>
            </div>
            <span className="mono muted">见下方</span>
          </div>
        </div>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>运行软件资源</h2>
            <p>镜像与磁盘占用由各运行软件自己的 CLI 报告。</p>
          </div>
        </div>
        {(backends ?? []).filter((backend) => backend.status.state === 'ready').length === 0 ? (
          <p className="muted">没有可用的运行软件。</p>
        ) : (
          <div className="cards">
            {(backends ?? [])
              .filter((backend) => backend.status.state === 'ready')
              .map((backend) => {
                const resource = resources[backend.kind]
                return (
                  <article className="card" key={backend.kind}>
                    <div className="card-head">
                      <h3>{backend.name}</h3>
                      <span className="mono muted">{backend.kind}</span>
                    </div>
                    {!resource ? (
                      <p className="muted">读取中…</p>
                    ) : resource.error ? (
                      <p className="card-note">{resource.error}</p>
                    ) : (
                      <>
                        <p className="card-note">
                          镜像 {resource.images.length} 个
                          {resource.images.length > 0 ? `：${resource.images.slice(0, 3).join('、')}` : ''}
                        </p>
                        <pre className="resource-disk">{resource.disk}</pre>
                        <div className="snapshot-create">
                          <input
                            value={imageDrafts[backend.kind] ?? ''}
                            onChange={(event) =>
                              setImageDrafts((previous) => ({
                                ...previous,
                                [backend.kind]: event.target.value,
                              }))
                            }
                            placeholder="拉取镜像，例如 alpine:3.20"
                            spellCheck={false}
                          />
                          <button
                            className="ghost"
                            onClick={() => void pull(backend.kind)}
                            disabled={pulling === backend.kind}
                          >
                            {pulling === backend.kind ? '拉取中…' : '拉取'}
                          </button>
                        </div>
                        <div className="snapshot-create">
                          <input
                            value={tarDrafts[backend.kind] ?? ''}
                            onChange={(event) =>
                              setTarDrafts((previous) => ({
                                ...previous,
                                [backend.kind]: event.target.value,
                              }))
                            }
                            placeholder="本地 OCI/tar 路径"
                            spellCheck={false}
                          />
                          <button
                            className="ghost"
                            onClick={() => void loadImage(backend.kind)}
                            disabled={loadingImage === backend.kind}
                          >
                            {loadingImage === backend.kind ? '导入中…' : '导入 tar'}
                          </button>
                        </div>
                        <button
                          className="ghost"
                          onClick={() => void prune(backend.kind)}
                          disabled={pruning === backend.kind}
                        >
                          {pruning === backend.kind ? '清理中…' : '清理未使用'}
                        </button>
                      </>
                    )}
                  </article>
                )
              })}
          </div>
        )}
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>应用设置</h2>
            <p>MART 自己的设置。</p>
          </div>
          <button className="ghost" onClick={onSetup}>初始化 / 添加 Agent</button>
        </div>
        <div className="settings-list">
          <div className="setting-row">
            <div>
              <strong>外观</strong>
              <span className="muted">中性黑白灰，跟随系统，也可在标题栏手动切换</span>
            </div>
            <span className="mono muted">{appearance}</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>连接安全</strong>
              <span className="muted">首次连接环境时记录证书指纹，之后自动核对</span>
            </div>
            <span className="mono muted">已启用</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>匿名统计</strong>
              <span className="muted">默认关闭；开启后只上报数量类信息，不包含任何内容</span>
            </div>
            <span className="mono muted">关闭</span>
          </div>
          <div className="setting-row">
            <div>
              <strong>版本</strong>
              <span className="muted">{host ? `${host.os} · ${host.arch}` : '—'}</span>
            </div>
            <span className="mono">{host ? `v${host.appVersion}` : '—'}</span>
          </div>
        </div>
      </section>
    </div>
  )
}
