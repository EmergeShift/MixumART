import { useEffect, useState } from 'react'

import {
  errorText,
  loadAgentProfiles,
  removeAgentProfile,
  saveAgentProfile,
  type AgentProfile,
} from '../api'

const MODES = [
  {
    title: '直接使用',
    summary: '外部 Agent 自己动手',
    detail:
      '例如 Codex 直接操作这个环境：打开终端、运行命令、查看窗口。环境里不运行我们提供的助手。',
  },
  {
    title: '派发任务',
    summary: '外部 Agent 把活交给环境里的助手',
    detail:
      'Codex 只负责安排任务，真正干活的是环境里的助手。模型密钥使用你自己的（BYOK），不会进入快照。',
  },
  {
    title: '完全隔离',
    summary: '一个 Agent 一个环境',
    detail:
      '把一个 Agent 的登录凭据放进专属环境，其它 Agent 看不到、也用不了。适合互不信任的场景。',
  },
]

const CORES = [
  { key: 'mixum', name: '内置助手', note: '推荐。默认自带，与快照、接管配合最好' },
  { key: 'pi', name: 'Pi', note: '使用 Pi 的执行引擎' },
  { key: 'deepseek', name: 'DeepSeek', note: '使用 DeepSeek 的 Agent 框架' },
  { key: 'codex', name: 'Codex', note: '让 Codex 在环境里执行' },
]

const KINDS = ['Codex', 'Claude Code', 'Cursor', '其它 MAP Agent']

const emptyDraft = (): AgentProfile => ({
  id: crypto.randomUUID(),
  name: '',
  kind: KINDS[0],
  mode: 'as-tool',
  core: 'mixum',
  command: '',
})

export function Agents() {
  const [profiles, setProfiles] = useState<AgentProfile[] | null>(null)
  const [editing, setEditing] = useState<AgentProfile | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const load = async () => {
    setError(null)
    try {
      setProfiles(await loadAgentProfiles())
    } catch (err) {
      setError(errorText(err))
      setProfiles([])
    }
  }

  useEffect(() => {
    void load()
  }, [])

  const save = async () => {
    if (!editing) return
    if (!editing.name.trim()) {
      setError('Agent 名称不能为空')
      return
    }
    setSaving(true)
    setError(null)
    try {
      await saveAgentProfile({ ...editing, name: editing.name.trim() })
      setEditing(null)
      await load()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setSaving(false)
    }
  }

  const remove = async (profile: AgentProfile) => {
    if (!window.confirm(`删除 Agent 配置 ${profile.name}？`)) return
    try {
      await removeAgentProfile(profile.id)
      await load()
    } catch (err) {
      setError(errorText(err))
    }
  }

  return (
    <div className="agents-page">
      <section>
        <div className="section-head">
          <div>
            <h2>Agent 怎么使用 MART</h2>
            <p>MART 自己不运行 Agent。你可以选择外部 Agent 和环境里的助手如何分工。</p>
          </div>
        </div>
        <div className="mode-grid">
          {MODES.map((mode) => (
            <article className="mode-card" key={mode.title}>
              <h3>{mode.summary}</h3>
              <p>{mode.detail}</p>
            </article>
          ))}
        </div>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>本机 Agent 配置</h2>
            <p>
              这里只保存启动方式和工作模式；凭据由外部 Agent 或系统钥匙串管理，
              MART 不保存登录令牌。
            </p>
          </div>
          <button className="ghost" onClick={() => setEditing(emptyDraft())}>
            添加 Agent
          </button>
        </div>
        {error && <div className="error">{error}</div>}
        {editing && (
          <section className="connect-panel" aria-label="Agent 配置">
            <div className="section-head">
              <div>
                <h3>{editing.name || '新 Agent'}</h3>
                <p>配置会保存在本机应用数据目录，不进入环境快照。</p>
              </div>
              <button className="ghost" onClick={() => setEditing(null)}>
                取消
              </button>
            </div>
            <div className="connect-grid">
              <div className="field">
                <label htmlFor="agent-name">名称</label>
                <input
                  id="agent-name"
                  value={editing.name}
                  onChange={(event) => setEditing({ ...editing, name: event.target.value })}
                  placeholder="例如 Codex"
                  spellCheck={false}
                />
              </div>
              <div className="field">
                <label htmlFor="agent-kind">类型</label>
                <select
                  id="agent-kind"
                  value={editing.kind}
                  onChange={(event) => setEditing({ ...editing, kind: event.target.value })}
                >
                  {KINDS.map((kind) => (
                    <option key={kind}>{kind}</option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="agent-mode">使用方式</label>
                <select
                  id="agent-mode"
                  value={editing.mode}
                  onChange={(event) => setEditing({ ...editing, mode: event.target.value })}
                >
                  <option value="as-tool">直接使用</option>
                  <option value="sub-agent">派发任务</option>
                  <option value="isolated">完全隔离</option>
                </select>
              </div>
              <div className="field">
                <label htmlFor="agent-core">环境内执行引擎</label>
                <select
                  id="agent-core"
                  value={editing.core}
                  onChange={(event) => setEditing({ ...editing, core: event.target.value })}
                >
                  {CORES.map((core) => (
                    <option key={core.key} value={core.key}>
                      {core.name}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="agent-command">启动命令（可选）</label>
                <input
                  id="agent-command"
                  className="mono"
                  value={editing.command}
                  onChange={(event) => setEditing({ ...editing, command: event.target.value })}
                  placeholder="例如 codex"
                  spellCheck={false}
                />
              </div>
            </div>
            <div className="connect-actions">
              <button className="primary" onClick={() => void save()} disabled={saving}>
                {saving ? '保存中…' : '保存配置'}
              </button>
            </div>
          </section>
        )}
        {profiles === null ? (
          <p className="muted">读取中…</p>
        ) : profiles.length === 0 ? (
          <div className="empty-panel">
            <h3>还没有 Agent 配置</h3>
            <p>添加一个外部 Agent，记录它使用哪个环境、以什么方式工作。</p>
          </div>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Agent</th>
                <th>类型</th>
                <th>使用方式</th>
                <th>执行引擎</th>
                <th>启动命令</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {profiles.map((profile) => (
                <tr key={profile.id}>
                  <td>{profile.name}</td>
                  <td className="muted">{profile.kind}</td>
                  <td>
                    <span className="tag">
                      {profile.mode === 'as-tool'
                        ? '直接使用'
                        : profile.mode === 'sub-agent'
                          ? '派发任务'
                          : '完全隔离'}
                    </span>
                  </td>
                  <td className="muted">
                    {CORES.find((core) => core.key === profile.core)?.name ?? profile.core}
                  </td>
                  <td className="mono muted">{profile.command || '—'}</td>
                  <td className="row-actions">
                    <button className="ghost" onClick={() => setEditing(profile)}>
                      编辑
                    </button>
                    <button className="ghost" onClick={() => void remove(profile)}>
                      删除
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>登录凭据</h2>
            <p>凭据由外部 Agent 自己或系统钥匙串管理，不进入 MART 配置文件和快照。</p>
          </div>
        </div>
        <div className="callout">
          <div>
            <strong>MART 不代管 Agent 登录令牌</strong>
            <p>
              需要隔离凭据时，请把 Agent 放进专属环境，并由环境内的 Agent 自己完成登录。
              这样切换 Agent 不会把令牌带进别的工作区。
            </p>
          </div>
        </div>
      </section>
    </div>
  )
}
