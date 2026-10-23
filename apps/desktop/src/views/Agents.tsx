import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

import {
  agentCatalog,
  agentEnvironments,
  agentInstall,
  agentPrompt,
  agentRemoteInstall,
  agentRemoteStatus,
  agentStatus,
  connectEnvironment,
  deleteAgentCredential,
  errorText,
  gatewayNodes,
  getAgentCredential,
  hasAgentCredential,
  loadAgentProfiles,
  loadEnvironmentConnections,
  runEnvironmentCommand,
  removeAgentProfile,
  saveAgentProfile,
  setAgentCredential,
  type AgentEnvironmentInfo,
  type AgentInfo,
  type AgentInstallEvent,
  type AgentProfile,
  type AgentStatus,
} from '../api'
import type { CommandResult } from '../api'
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

const REMOTE_TARGET = '__remote__'

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
  const [remoteAddr, setRemoteAddr] = useState(() => localStorage.getItem('mixum.map.addr') ?? '')
  const [remoteFp, setRemoteFp] = useState(() => localStorage.getItem('mixum.map.fp') ?? '')
  const [remoteNode, setRemoteNode] = useState(() => localStorage.getItem('mixum.map.node') ?? '')
  const [remoteToken, setRemoteToken] = useState('')
  const [remoteNodes, setRemoteNodes] = useState<string[]>([])
  const [remoteBusy, setRemoteBusy] = useState(false)
  const [remoteConnected, setRemoteConnected] = useState(false)
  const [agentInstallState, setAgentInstallState] = useState<string | null>(null)
  const installIdRef = useRef<string | null>(null)

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

  const remoteAddrRef = useRef(remoteAddr)
  const remoteKeyRef = useRef(`${remoteAddr}|${remoteNode}`)
  const agentEnvRef = useRef(agentEnvId)
  const prevRemoteAddr = useRef(remoteAddr)

  useEffect(() => {
    remoteAddrRef.current = remoteAddr
    if (prevRemoteAddr.current === remoteAddr) return
    prevRemoteAddr.current = remoteAddr
    // 换 gateway 必须清掉旧 bearer token/指纹/节点与安装结果,避免把 A 的凭据
    // 发到 B,也避免旧地址的异步响应回写到新地址。
    remoteKeyRef.current = `${remoteAddr}|`
    setRemoteToken('')
    setRemoteFp('')
    setRemoteNode('')
    setRemoteNodes([])
    setRemoteConnected(false)
    setRemoteBusy(false)
    setAgentProbing(false)
    setAgentRows(null)
    setAgentLines(null)
    setAgentError(null)
    setAgentInstalling(null)
    setAgentInstallName('')
    localStorage.removeItem('mixum.map.node')
    localStorage.removeItem('mixum.map.fp')
  }, [remoteAddr])

  const isRemote = agentEnvId === REMOTE_TARGET

  const refreshRemoteStatus = async (
    addr: string,
    token: string,
    node: string,
    fingerprint: string,
  ) => {
    const key = `${addr.trim()}|${node}`
    setAgentProbing(true)
    setAgentError(null)
    try {
      const rows = await agentRemoteStatus(
        addr.trim(),
        token.trim(),
        node,
        fingerprint.trim() || undefined,
      )
      if (remoteKeyRef.current === key) setAgentRows(rows)
    } catch (err) {
      if (remoteKeyRef.current === key) {
        setAgentError(errorText(err))
        setAgentRows(null)
      }
    } finally {
      if (remoteKeyRef.current === key) setAgentProbing(false)
    }
  }

  const connectRemote = async () => {
    const addr = remoteAddr.trim()
    const token = remoteToken.trim()
    const fingerprint = remoteFp.trim()
    if (!addr || !token) {
      setAgentError('请填写远程 gateway 地址和访问令牌')
      return
    }
    let key = `${addr}|${remoteNode}`
    setRemoteBusy(true)
    setAgentError(null)
    try {
      const nodes = await gatewayNodes(addr, token, fingerprint || undefined)
      if (remoteAddrRef.current !== addr) return
      setRemoteNodes(nodes)
      const next = nodes.includes(remoteNode) ? remoteNode : (nodes[0] ?? '')
      key = `${addr}|${next}`
      remoteKeyRef.current = key
      setRemoteNode(next)
      setRemoteConnected(true)
      localStorage.setItem('mixum.map.addr', addr)
      if (fingerprint) localStorage.setItem('mixum.map.fp', fingerprint)
      else localStorage.removeItem('mixum.map.fp')
      if (next) {
        localStorage.setItem('mixum.map.node', next)
        await refreshRemoteStatus(addr, token, next, fingerprint)
      } else {
        setAgentRows(null)
        setAgentError('gateway 上没有在线节点')
      }
    } catch (err) {
      if (remoteKeyRef.current === key) {
        setAgentError(errorText(err))
        setRemoteConnected(false)
      }
    } finally {
      if (remoteKeyRef.current === key) setRemoteBusy(false)
    }
  }

  const refreshAgentStatus = async (env?: AgentEnvironmentInfo) => {
    if (agentEnvId === REMOTE_TARGET) {
      if (!remoteConnected || !remoteNode) {
        setAgentError('请先连接 gateway 并选择节点')
        return
      }
      await refreshRemoteStatus(remoteAddr, remoteToken, remoteNode, remoteFp)
      return
    }
    const target = env ?? agentEnvs.find((item) => item.id === agentEnvId)
    if (!target) return
    const envId = target.id
    setAgentProbing(true)
    setAgentError(null)
    try {
      await ensureAgentConnection(target)
      const rows = await agentStatus(envId)
      if (agentEnvRef.current === envId) setAgentRows(rows)
    } catch (err) {
      if (agentEnvRef.current === envId) {
        setAgentError(errorText(err))
        setAgentRows(null)
      }
    } finally {
      if (agentEnvRef.current === envId) setAgentProbing(false)
    }
  }

  const applyInstallResult = (result: CommandResult) => {
    setAgentInstallState(result.state)
    setAgentLines((previous) => {
      const merged = [...(previous ?? [])]
      for (const line of result.lines) {
        if (!merged.includes(line)) merged.push(line)
      }
      const stateLine = `[${result.state}]`
      if (!merged.includes(stateLine)) merged.push(stateLine)
      return merged
    })
    if (result.error) setAgentError(result.error)
  }

  const installAgent = async (name: string) => {
    if (agentEnvId === REMOTE_TARGET) {
      if (!remoteConnected || !remoteNode) {
        setAgentError('请先连接 gateway 并选择节点')
        return
      }
      if (
        !window.confirm(
          `在远程节点「${remoteNode}」安装 ${name}？

安装会在节点里联网下载，可能需要几分钟；完成后提示词写入 ~/.mixum/agents/${name}/PROMPT.md。`,
        )
      ) {
        return
      }
      const label = `${remoteAddr} · ${remoteNode}`
      const installId = crypto.randomUUID()
      const installKey = `${remoteAddr.trim()}|${remoteNode}`
      installIdRef.current = installId
      setAgentInstalling(name)
      setAgentInstallName(label)
      setAgentInstallState(null)
      setAgentError(null)
      setAgentLines([`[安装] ${name} → ${label}`, '日志会实时显示；完成后自动刷新状态。'])
      try {
        const result = await agentRemoteInstall(
          remoteAddr.trim(),
          remoteToken.trim(),
          remoteNode,
          name,
          remoteFp.trim() || undefined,
        )
        if (
          installIdRef.current !== installId ||
          remoteKeyRef.current !== installKey
        ) {
          return
        }
        applyInstallResult(result)
        if (result.state !== 'completed') {
          setAgentError(result.error?.trim() || `安装未完成（${result.state}）`)
        } else {
          await refreshRemoteStatus(remoteAddr, remoteToken, remoteNode, remoteFp)
        }
      } catch (err) {
        if (installIdRef.current === installId) {
          setAgentLines((previous) => [...(previous ?? []), '[failed]'])
          setAgentError(errorText(err))
        }
      } finally {
        if (installIdRef.current === installId) {
          installIdRef.current = null
          setAgentInstalling(null)
        }
      }
      return
    }
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
    const installId = crypto.randomUUID()
    const envId = target.id
    installIdRef.current = installId
    setAgentInstalling(name)
    setAgentInstallName(target.name)
    setAgentInstallState(null)
    setAgentError(null)
    setAgentLines([`[安装] ${name} → ${target.name}`, `环境: ${target.addr}`, '日志会实时显示；完成后自动刷新状态。'])
    try {
      await ensureAgentConnection(target)
      const result = await agentInstall(target.id, name)
      if (
        installIdRef.current !== installId ||
        agentEnvRef.current !== envId
      ) {
        return
      }
      applyInstallResult(result)
      if (result.state !== 'completed') {
        setAgentError(result.error?.trim() || `安装未完成（${result.state}）`)
      } else {
        await refreshAgentStatus(target)
      }
    } catch (err) {
      if (installIdRef.current === installId) {
        setAgentLines((previous) => [...(previous ?? []), '[failed]'])
        setAgentError(errorText(err))
      }
    } finally {
      if (installIdRef.current === installId) {
        installIdRef.current = null
        setAgentInstalling(null)
      }
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
        setAgentEnvId((current) => current || first?.id || REMOTE_TARGET)
        if (first) void refreshAgentStatus(first)
      })
      .catch((err: unknown) => setAgentError(errorText(err)))
  }, [])

  useEffect(() => {
    const savedAddr = localStorage.getItem('mixum.map.addr')
    if (!savedAddr) return
    void getAgentCredential(`gateway-device:${savedAddr}`)
      .then((saved) => {
        if (saved) setRemoteToken((current) => current || saved)
      })
      .catch(() => undefined)
  }, [])

  useEffect(() => {
    let dispose: (() => void) | undefined
    listen<AgentInstallEvent>('agent-install', (event) => {
      const payload = event.payload
      if (!payload.installId || payload.installId !== installIdRef.current) return
      if (payload.kind === 'log' && payload.line) {
        setAgentLines((previous) => [...(previous ?? []), payload.line as string])
        return
      }
      if (payload.kind === 'state') {
        setAgentInstallState(payload.state ?? null)
        if (payload.message) setAgentError(payload.message)
      }
    })
      .then((unlisten) => {
        dispose = unlisten
      })
      .catch(() => undefined)
    return () => dispose?.()
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
        <div className="chips" style={{ alignItems: 'flex-end' }}>
          <div className="field">
            <label htmlFor="agent-env">目标环境</label>
            <select
              id="agent-env"
              value={agentEnvId}
              onChange={(event) => {
                const value = event.target.value
                setAgentEnvId(value)
                setAgentRows(null)
                setAgentError(null)
                if (value !== REMOTE_TARGET) {
                  const next = agentEnvs.find((item) => item.id === value)
                  if (next) void refreshAgentStatus(next)
                }
              }}
              disabled={agentProbing || !!agentInstalling || remoteBusy}
            >
              {agentEnvs.map((env) => (
                <option key={env.id} value={env.id}>
                  {env.name} · {env.addr}
                </option>
              ))}
              <option value={REMOTE_TARGET}>远程节点（gateway）</option>
            </select>
          </div>
          {isRemote && (
            <>
              <div className="field">
                <label htmlFor="agent-remote-addr">Gateway 地址</label>
                <input
                  id="agent-remote-addr"
                  className="mono"
                  value={remoteAddr}
                  onChange={(event) => {
                    remoteAddrRef.current = event.target.value
                    setRemoteAddr(event.target.value)
                  }}
                  placeholder="127.0.0.1:17701"
                  spellCheck={false}
                />
              </div>
              <div className="field">
                <label htmlFor="agent-remote-token">访问令牌</label>
                <input
                  id="agent-remote-token"
                  className="mono"
                  type="password"
                  value={remoteToken}
                  onChange={(event) => setRemoteToken(event.target.value)}
                  placeholder="map1.…（也支持已配对设备令牌）"
                  spellCheck={false}
                />
              </div>
              <div className="field">
                <label htmlFor="agent-remote-fp">证书指纹（可选）</label>
                <input
                  id="agent-remote-fp"
                  className="mono"
                  value={remoteFp}
                  onChange={(event) => setRemoteFp(event.target.value)}
                  placeholder="首次连接可留空"
                  spellCheck={false}
                />
              </div>
              <button
                className="ghost"
                onClick={() => void connectRemote()}
                disabled={remoteBusy || agentProbing || !!agentInstalling}
              >
                {remoteBusy ? '连接中…' : '连接并刷新节点'}
              </button>
              {remoteNodes.length > 0 && (
                <div className="field">
                  <label htmlFor="agent-remote-node">节点</label>
                  <select
                    id="agent-remote-node"
                    value={remoteNode}
                    onChange={(event) => {
                      const next = event.target.value
                      setRemoteNode(next)
                      localStorage.setItem('mixum.map.node', next)
                      setAgentRows(null)
                      if (remoteConnected && remoteToken.trim()) {
                        void refreshRemoteStatus(remoteAddr, remoteToken, next, remoteFp)
                      }
                    }}
                    disabled={agentProbing || !!agentInstalling}
                  >
                    {remoteNodes.map((name) => (
                      <option key={name} value={name}>
                        {name}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </>
          )}
          <button
            className="ghost"
            onClick={() => void refreshAgentStatus()}
            disabled={agentProbing || !!agentInstalling || !agentEnvId}
          >
            {agentProbing ? '探测中…' : '刷新安装状态'}
          </button>
        </div>
        {agentEnvs.length === 0 && !isRemote && (
          <p className="mode-note">
            没有运行中的 mixumOS 环境。可以先选择「远程节点（gateway）」，或到「环境」页启动一个环境。
          </p>
        )}
        {isRemote && (
          <p className="mode-note">
            远程安装通过 gateway 路由到节点，需要 task.run 权限；配对设备令牌权限不足时请粘贴用户/Admin 令牌。
          </p>
        )}
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
              安装会联网下载，日志实时显示，完成后自动刷新状态。任务在环境内执行，
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
                    <h3>{agentInstallName || '环境'} · 安装输出{agentInstallState ? ` · ${agentInstallState}` : ''}</h3>
                    <p>关闭不会终止已提交的安装任务。</p>
                  </div>
                  <button className="ghost" onClick={() => setAgentLines(null)}>
                    关闭
                  </button>
                </div>
                <pre className="terminal-output">{agentLines.join('\n')}</pre>
              </section>
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
