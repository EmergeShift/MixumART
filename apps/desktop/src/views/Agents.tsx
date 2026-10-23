import { useEffect, useState } from 'react'

import {
  agentCatalog,
  agentEnvironments,
  agentInstall,
  agentPrompt,
  agentStatus,
  connectEnvironment,
  deleteAgentCredential,
  errorText,
  hasAgentCredential,
  loadAgentProfiles,
  loadEnvironmentConnections,
  runEnvironmentCommand,
  removeAgentProfile,
  saveAgentProfile,
  setAgentCredential,
  type AgentEnvironmentInfo,
  type AgentInfo,
  type AgentProfile,
  type AgentStatus,
} from '../api'
import type { ConnectionInfo } from '../types'

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
  environmentId: '',
})

export function Agents() {
  const [profiles, setProfiles] = useState<AgentProfile[] | null>(null)
  const [editing, setEditing] = useState<AgentProfile | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [credentialProfile, setCredentialProfile] = useState<AgentProfile | null>(null)
  const [secret, setSecret] = useState('')
  const [credentialSaved, setCredentialSaved] = useState<boolean | null>(null)
  const [credentialBusy, setCredentialBusy] = useState(false)
  const [connections, setConnections] = useState<Record<string, ConnectionInfo>>({})
  const [launching, setLaunching] = useState<string | null>(null)
  const [launchLines, setLaunchLines] = useState<string[] | null>(null)
  const [launchName, setLaunchName] = useState('')
  const [agentEnvs, setAgentEnvs] = useState<AgentEnvironmentInfo[]>([])
  const [agentEnvId, setAgentEnvId] = useState('')
  const [agentCatalogList, setAgentCatalogList] = useState<AgentInfo[]>([])
  const [agentRows, setAgentRows] = useState<AgentStatus[] | null>(null)
  const [agentProbing, setAgentProbing] = useState(false)
  const [agentInstalling, setAgentInstalling] = useState<string | null>(null)
  const [agentLines, setAgentLines] = useState<string[] | null>(null)
  const [agentInstallName, setAgentInstallName] = useState('')
  const [agentError, setAgentError] = useState<string | null>(null)
  const [copiedAgent, setCopiedAgent] = useState<string | null>(null)
  const [promptFallback, setPromptFallback] = useState<{ name: string; text: string } | null>(null)

  const load = async () => {
    setError(null)
    try {
      setProfiles(await loadAgentProfiles())
    } catch (err) {
      setError(errorText(err))
      setProfiles([])
    }
  }

  const ensureAgentConnection = async (env: AgentEnvironmentInfo) => {
    if (connections[env.id]) return connections[env.id]
    const connection = await connectEnvironment(env.kind, env.id, env.addr)
    setConnections((previous) => ({ ...previous, [env.id]: connection }))
    return connection
  }

  const refreshAgentStatus = async (env?: AgentEnvironmentInfo) => {
    const target = env ?? agentEnvs.find((item) => item.id === agentEnvId)
    if (!target) return
    setAgentProbing(true)
    setAgentError(null)
    try {
      await ensureAgentConnection(target)
      setAgentRows(await agentStatus(target.id))
    } catch (err) {
      setAgentError(errorText(err))
      setAgentRows(null)
    } finally {
      setAgentProbing(false)
    }
  }

  const installAgent = async (name: string) => {
    const target = agentEnvs.find((item) => item.id === agentEnvId)
    if (!target) {
      setAgentError('请先选择一个运行中的环境')
      return
    }
    if (
      !window.confirm(
        `在「${target.name}」内安装 ${name}？\n\n安装会联网下载，可能需要几分钟；完成后提示词写入 ~/.mixum/agents/${name}/PROMPT.md。`,
      )
    ) {
      return
    }
    setAgentInstalling(name)
    setAgentInstallName(target.name)
    setAgentError(null)
    setAgentLines([`[安装] ${name} → ${target.name}`, `环境: ${target.addr}`, '正在安装，完成后会显示日志并刷新状态…'])
    try {
      await ensureAgentConnection(target)
      const result = await agentInstall(target.id, name)
      setAgentLines((previous) => [...(previous ?? []), ...result.lines, `[${result.state}]`])
      if (result.error) {
        setAgentError(result.error)
      } else {
        await refreshAgentStatus(target)
      }
    } catch (err) {
      setAgentError(errorText(err))
    } finally {
      setAgentInstalling(null)
    }
  }

  const copyAgentPrompt = async (name: string) => {
    try {
      const text = await agentPrompt(name)
      let copied = false
      try {
        await navigator.clipboard.writeText(text)
        copied = true
      } catch {
        const area = document.createElement('textarea')
        area.value = text
        document.body.appendChild(area)
        area.select()
        copied = document.execCommand('copy')
        area.remove()
      }
      if (!copied) {
        setPromptFallback({ name, text })
        return
      }
      setPromptFallback(null)
      setCopiedAgent(name)
      window.setTimeout(
        () => setCopiedAgent((current) => (current === name ? null : current)),
        2000,
      )
    } catch (err) {
      setAgentError(errorText(err))
    }
  }

  useEffect(() => {
    void load()
    void loadEnvironmentConnections()
      .then(setConnections)
      .catch((err: unknown) => setError(errorText(err)))
    void agentCatalog()
      .then(setAgentCatalogList)
      .catch((err: unknown) => setAgentError(errorText(err)))
    void agentEnvironments()
      .then((environments) => {
        setAgentEnvs(environments)
        const first = environments[0]
        if (first) {
          setAgentEnvId((current) => current || first.id)
          void refreshAgentStatus(first)
        }
      })
      .catch((err: unknown) => setAgentError(errorText(err)))
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

  const openCredential = async (profile: AgentProfile) => {
    setCredentialProfile(profile)
    setSecret('')
    setCredentialSaved(null)
    try {
      setCredentialSaved(await hasAgentCredential(profile.id))
    } catch (err) {
      setError(errorText(err))
    }
  }

  const saveCredential = async () => {
    if (!credentialProfile || !secret) return
    setCredentialBusy(true)
    setError(null)
    try {
      await setAgentCredential(credentialProfile.id, secret)
      setCredentialSaved(true)
      setSecret('')
    } catch (err) {
      setError(errorText(err))
    } finally {
      setCredentialBusy(false)
    }
  }

  const removeCredential = async () => {
    if (!credentialProfile) return
    if (!window.confirm(`删除 ${credentialProfile.name} 的系统钥匙串凭据？`)) return
    setCredentialBusy(true)
    try {
      await deleteAgentCredential(credentialProfile.id)
      setCredentialSaved(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setCredentialBusy(false)
    }
  }

  const startAgent = async (profile: AgentProfile) => {
    if (!profile.command.trim()) {
      setError('这个 Agent 还没有启动命令')
      return
    }
    if (!profile.environmentId || !connections[profile.environmentId]) {
      setError('请先给 Agent 绑定一个已连接的环境')
      return
    }
    setLaunching(profile.id)
    setLaunchName(profile.name)
    setError(null)
    setLaunchLines([`[启动] ${profile.name}`, `环境: ${connections[profile.environmentId].addr}`, `$ ${profile.command}`])
    try {
      const result = await runEnvironmentCommand(profile.environmentId, profile.command)
      setLaunchLines((previous) => [...(previous ?? []), ...result.lines])
      if (result.error) setError(result.error)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setLaunching(null)
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
            <h2>环境内一键安装</h2>
            <p>
              在运行中的 mixumOS 环境里安装主流 Agent；安装脚本在环境内执行，提示词写到{' '}
              <span className="mono">{'~/.mixum/agents/<name>/PROMPT.md'}</span>。
              也可以只复制提示词，挂到任意外部 Agent 上。
            </p>
          </div>
        </div>
        {agentError && <div className="error">{agentError}</div>}
        {agentEnvs.length === 0 ? (
          <p className="mode-note">
            没有运行中的 mixumOS 环境。先到「环境」页启动一个带 agentd 的环境，再回来安装。
          </p>
        ) : (
          <>
            <div className="chips" style={{ alignItems: 'flex-end' }}>
              <div className="field">
                <label htmlFor="agent-env">目标环境</label>
                <select
                  id="agent-env"
                  value={agentEnvId}
                  onChange={(event) => {
                    const next = agentEnvs.find((item) => item.id === event.target.value)
                    setAgentEnvId(event.target.value)
                    setAgentRows(null)
                    if (next) void refreshAgentStatus(next)
                  }}
                  disabled={agentProbing || !!agentInstalling}
                >
                  {agentEnvs.map((env) => (
                    <option key={env.id} value={env.id}>
                      {env.name} · {env.addr}
                    </option>
                  ))}
                </select>
              </div>
              <button
                className="ghost"
                onClick={() => void refreshAgentStatus()}
                disabled={agentProbing || !!agentInstalling || !agentEnvId}
              >
                {agentProbing ? '探测中…' : '刷新安装状态'}
              </button>
            </div>
            <div className="table-scroll">
              <table className="table">
                <thead>
                  <tr>
                    <th>Agent</th>
                    <th>说明</th>
                    <th>状态</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {(agentRows ??
                    agentCatalogList.map((agent) => ({ ...agent, installed: false }))
                  ).map((row) => (
                    <tr key={row.name}>
                      <td className="mono">{row.name}</td>
                      <td>{row.summary}</td>
                      <td>{agentRows ? (row.installed ? '已安装' : '未安装') : '未知'}</td>
                      <td className="row-actions">
                        <button
                          className="ghost"
                          onClick={() => void copyAgentPrompt(row.name)}
                        >
                          {copiedAgent === row.name ? '已复制' : '复制提示词'}
                        </button>
                        <button
                          className="ghost"
                          onClick={() => void installAgent(row.name)}
                          disabled={!!agentInstalling || row.installed}
                        >
                          {agentInstalling === row.name ? '安装中…' : '安装'}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mode-note">
              安装会联网下载，可能需要几分钟；完成后自动刷新状态。任务在环境内执行，
              受快照、隔离与权限审批约束。
            </p>
            {promptFallback && (
              <section className="connect-panel" aria-label="Agent 提示词">
                <div className="section-head">
                  <div>
                    <h3>{promptFallback.name} · 提示词</h3>
                    <p>系统拒绝自动写入剪贴板，请全选手动复制。</p>
                  </div>
                  <button className="ghost" onClick={() => setPromptFallback(null)}>
                    关闭
                  </button>
                </div>
                <textarea
                  className="mono"
                  readOnly
                  rows={8}
                  style={{ width: '100%', resize: 'vertical' }}
                  value={promptFallback.text}
                />
              </section>
            )}
            {agentLines && (
              <section className="connect-panel terminal-panel" aria-label="Agent 安装输出">
                <div className="section-head">
                  <div>
                    <h3>{agentInstallName || '环境'} · 安装输出</h3>
                    <p>关闭不会终止已提交的安装任务。</p>
                  </div>
                  <button className="ghost" onClick={() => setAgentLines(null)}>
                    关闭
                  </button>
                </div>
                <pre className="terminal-output">{agentLines.join('\n')}</pre>
              </section>
            )}
          </>
        )}
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>本机 Agent 配置</h2>
            <p>
              这里保存启动方式和工作模式；登录凭据单独写入系统钥匙串，不进入
              MART 配置文件或环境快照。
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
              <div className="field">
                <label htmlFor="agent-environment">绑定环境</label>
                <select
                  id="agent-environment"
                  value={editing.environmentId ?? ''}
                  onChange={(event) => setEditing({ ...editing, environmentId: event.target.value })}
                >
                  <option value="">不绑定</option>
                  {Object.entries(connections).map(([id, connection]) => (
                    <option key={id} value={id}>
                      {connection.addr}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <div className="connect-actions">
              <button className="primary" onClick={() => void save()} disabled={saving}>
                {saving ? '保存中…' : '保存配置'}
              </button>
            </div>
          </section>
        )}
        {credentialProfile && (
          <section className="connect-panel" aria-label="Agent 凭据">
            <div className="section-head">
              <div>
                <h3>{credentialProfile.name} · 凭据</h3>
                <p>保存到系统钥匙串，不写入 connections.json 或快照。</p>
              </div>
              <button className="ghost" onClick={() => setCredentialProfile(null)}>
                关闭
              </button>
            </div>
            {credentialSaved !== null && (
              <p className="muted">
                {credentialSaved ? '钥匙串中已有凭据' : '钥匙串中还没有凭据'}
              </p>
            )}
            <div className="connect-grid">
              <div className="field">
                <label htmlFor="agent-secret">新凭据</label>
                <input
                  id="agent-secret"
                  type="password"
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                  placeholder="只写入系统钥匙串，不会回显"
                  spellCheck={false}
                />
              </div>
            </div>
            <div className="connect-actions">
              <button className="primary" onClick={() => void saveCredential()} disabled={credentialBusy || !secret}>
                {credentialBusy ? '保存中…' : '保存到钥匙串'}
              </button>
              <button className="ghost" onClick={() => void removeCredential()} disabled={credentialBusy}>
                删除凭据
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
          <div className="table-scroll"><table className="table">
            <thead>
              <tr>
                <th>Agent</th>
                <th>类型</th>
                <th>使用方式</th>
                <th>执行引擎</th>
                <th>环境</th>
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
                  <td className="mono muted">
                    {profile.environmentId
                      ? connections[profile.environmentId]?.addr ?? '连接已失效'
                      : '未绑定'}
                  </td>
                  <td className="mono muted">{profile.command || '—'}</td>
                  <td className="row-actions">
                    <button
                      className="ghost"
                      onClick={() => void startAgent(profile)}
                      disabled={launching === profile.id}
                    >
                      {launching === profile.id ? '启动中…' : '启动'}
                    </button>
                    <button className="ghost" onClick={() => void openCredential(profile)}>
                      凭据
                    </button>
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
          </table></div>
        )}
      </section>

      {launchLines && (
        <section className="connect-panel terminal-panel" aria-label="Agent 启动输出">
          <div className="section-head">
            <div>
              <h3>{launchName} · 启动输出</h3>
              <p>一次性启动；凭据仍由环境/外部 Agent 负责。</p>
            </div>
            <button className="ghost" onClick={() => setLaunchLines(null)}>
              关闭
            </button>
          </div>
          <pre className="terminal-output">{launchLines.join('\n')}</pre>
        </section>
      )}

      <section>
        <div className="section-head">
          <div>
            <h2>登录凭据</h2>
            <p>凭据写入系统钥匙串；MART 配置文件与快照不保存登录令牌。</p>
          </div>
        </div>
        <div className="callout">
          <div>
            <strong>凭据只在系统钥匙串里</strong>
            <p>
              需要隔离凭据时，请把 Agent 放进专属环境；MART 只保存启动配置，
              不会把令牌带进别的工作区。
            </p>
          </div>
        </div>
      </section>
    </div>
  )
}
