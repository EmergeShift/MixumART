import { useEffect, useMemo, useState } from 'react'

import {
  errorText,
  setupApply,
  setupInstallRunner,
  setupStartRunner,
  setupPlan,
} from '../api'
import type { SetupPlan, SetupResult, SetupLanguage } from '../types'

type Props = {
  onDone: () => void
}

const COPY = {
  'zh-CN': {
    title: '初始化',
    sub: '选择语言、环境系统和要接入的 Agent。CLI 与桌面端使用同一份设置。',
    language: '语言',
    languageHint: '之后可以在设置里再次进入这里。',
    runner: '运行环境',
    runnerHint: '选择一个 VM Runner。未安装的会给出安装命令。',
    ready: '可用',
    install: '安装',
    start: '启动',
    installing: '安装中…',
    image: 'mixumOS',
    imageHint: '使用本地镜像，或输入镜像引用后拉取。',
    pull: '拉取',
    localImage: '本地镜像',
    existingEnv: '已有环境',
    agents: 'Agent',
    agentsHint: '把 mixum MCP 注册到这些 Agent。已安装的会更新启动路径。',
    configured: '已配置',
    needsUpdate: '需要更新',
    missing: '未检测到',
    invalid: '配置损坏',
    next: '继续',
    back: '返回',
    apply: '应用',
    applying: '应用中…',
    done: '完成',
    addMore: '继续添加 Agent',
    restart: '重启对应 Agent 后生效。',
    noRunner: '没有可用的 VM Runner。',
    noImage: '没有选择镜像；可以稍后在设置里选择。',
    error: '失败',
    success: '已配置',
  },
  en: {
    title: 'Setup',
    sub: 'Choose a language, runtime and the agents to connect. CLI and desktop share the same settings.',
    language: 'Language',
    languageHint: 'You can return here later from Settings.',
    runner: 'Runtime',
    runnerHint: 'Choose a VM runner. Missing ones show an install command.',
    ready: 'ready',
    install: 'Install',
    start: 'Start',
    installing: 'Installing…',
    image: 'mixumOS',
    imageHint: 'Use a local image, or enter a reference to pull.',
    pull: 'Pull',
    localImage: 'Local images',
    existingEnv: 'Existing environments',
    agents: 'Agents',
    agentsHint: 'Register the mixum MCP with these agents. Existing entries are updated.',
    configured: 'configured',
    needsUpdate: 'needs update',
    missing: 'not detected',
    invalid: 'invalid config',
    next: 'Continue',
    back: 'Back',
    apply: 'Apply',
    applying: 'Applying…',
    done: 'Done',
    addMore: 'Add more agents',
    restart: 'Restart the affected agents after applying.',
    noRunner: 'No VM runner is available.',
    noImage: 'No image selected; you can pick one later in Settings.',
    error: 'Failed',
    success: 'Configured',
  },
} as const

function statusLabel(language: SetupLanguage, status: string) {
  const copy = COPY[language]
  switch (status) {
    case 'configured':
      return copy.configured
    case 'needs_update':
      return copy.needsUpdate
    case 'invalid':
      return copy.invalid
    default:
      return copy.missing
  }
}

export function Setup({ onDone }: Props) {
  const [plan, setPlan] = useState<SetupPlan | null>(null)
  const [step, setStep] = useState(0)
  const [language, setLanguage] = useState<SetupLanguage>('zh-CN')
  const [runtime, setRuntime] = useState<SetupPlan['runners'][number]['kind'] | null>(null)
  const [image, setImage] = useState('')
  const [environmentId, setEnvironmentId] = useState<string | null>(null)
  const [pull, setPull] = useState(false)
  const [clients, setClients] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [installing, setInstalling] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<SetupResult | null>(null)

  const copy = COPY[language]
  const mixumImages = useMemo(
    () => (plan?.images ?? []).filter((item) => item.reference.toLowerCase().includes('mixumos')),
    [plan],
  )

  const load = async () => {
    setError(null)
    try {
      const next = await setupPlan()
      setPlan(next)
      const savedLanguage = next.settings.language
      if (savedLanguage) setLanguage(savedLanguage)
      const savedRuntime =
        next.runners.find((item) => item.kind === next.settings.runtime && item.supported)?.kind ??
        next.runners.find((item) => item.supported && item.status.state === 'ready')?.kind ??
        next.runners.find((item) => item.supported)?.kind ??
        null
      setRuntime(savedRuntime)
      const savedImage = next.settings.image ?? mixumImages[0]?.reference ?? next.defaultImage
      setImage(savedImage)
      setEnvironmentId(next.settings.environmentId)
      setPull(!mixumImages.some((item) => item.reference === savedImage))
      const savedClients = next.settings.agents.length
        ? next.settings.agents
        : next.clients
            .filter((item) => item.status === 'configured' || item.status === 'needs_update')
            .map((item) => item.id)
      setClients(savedClients)
      if (next.settings.completed) setStep(2)
    } catch (err) {
      setError(errorText(err))
    }
  }

  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const install = async (kind: SetupPlan['runners'][number]['kind'], start: boolean) => {
    setInstalling(kind)
    setError(null)
    try {
      const output = start
        ? await setupStartRunner(kind)
        : await setupInstallRunner(kind)
      if (output) window.alert(output)
      await load()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setInstalling(null)
    }
  }

  const apply = async () => {
    if (!runtime) {
      setError(copy.noRunner)
      return
    }
    if (!image.trim()) {
      setError(copy.noImage)
      return
    }
    setBusy(true)
    setError(null)
    try {
      const next = await setupApply({
        language,
        runtime,
        image: image.trim() || null,
        pullImage: pull,
        environmentId,
        clients,
      })
      setResult(next)
      setStep(3)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  if (!plan) {
    return (
      <div className="setup-page">
        <section>
          <div className="section-head">
            <div>
              <h2>{copy.title}</h2>
              <p>{copy.sub}</p>
            </div>
          </div>
          {error ? <div className="error">{error}</div> : <p className="muted">…</p>}
        </section>
      </div>
    )
  }

  return (
    <div className="setup-page">
      <section>
        <div className="section-head">
          <div>
            <h2>{copy.title}</h2>
            <p>{copy.sub}</p>
          </div>
          <div className="setup-steps">
            {[0, 1, 2].map((index) => (
              <span key={index} className={step === index ? 'active' : step > index ? 'done' : ''}>
                {index + 1}
              </span>
            ))}
          </div>
        </div>
        {error && <div className="error">{error}</div>}
      </section>

      {step === 0 && (
        <section>
          <h3>{copy.language}</h3>
          <p className="muted">{copy.languageHint}</p>
          <div className="setup-options">
            {(['zh-CN', 'en'] as const).map((value) => (
              <button
                key={value}
                className={`option-card ${language === value ? 'selected' : ''}`}
                onClick={() => setLanguage(value)}
              >
                <strong>{value === 'zh-CN' ? '中文' : 'English'}</strong>
                <span>{value}</span>
              </button>
            ))}
          </div>
          <div className="setup-actions">
            <button className="primary" onClick={() => setStep(1)}>{copy.next}</button>
          </div>
        </section>
      )}

      {step === 1 && (
        <section>
          <h3>{copy.runner}</h3>
          <p className="muted">{copy.runnerHint}</p>
          <div className="setup-options">
            {plan.runners.filter((item) => item.supported).map((item) => (
              <button
                key={item.kind}
                className={`option-card ${runtime === item.kind ? 'selected' : ''}`}
                onClick={() => setRuntime(item.kind)}
              >
                <strong>{item.name}</strong>
                <span>
                  {item.status.state === 'ready'
                    ? item.status.version ?? copy.ready
                    : item.status.state === 'not_installed' || item.status.state === 'not_running'
                      ? item.status.hint
                      : item.status.reason}
                </span>
                {item.status.state !== 'ready' && (item.start || item.install) && (
                  <span
                    className="setup-install"
                    onClick={(event) => {
                      event.stopPropagation()
                      setRuntime(item.kind)
                      void install(item.kind, Boolean(item.start && !item.install))
                    }}
                  >
                    {installing === item.kind
                      ? copy.installing
                      : item.start
                        ? `${copy.start}: ${item.start.command.join(' ')}`
                        : `${copy.install}: ${item.install?.command.join(' ')}`}
                  </span>
                )}
              </button>
            ))}
          </div>
          {plan.runners.every((item) => !item.supported) && <p className="muted">{copy.noRunner}</p>}

          <h3>{copy.image}</h3>
          <p className="muted">{copy.imageHint}</p>
          {mixumImages.length > 0 && (
            <div className="setup-options">
              {mixumImages.map((item) => (
                <button
                  key={`${item.runtime}:${item.reference}`}
                  className={`option-card ${image === item.reference ? 'selected' : ''}`}
                  onClick={() => {
                    setImage(item.reference)
                    setRuntime(item.runtime)
                    setPull(false)
                    setEnvironmentId(null)
                  }}
                >
                  <strong>{item.reference}</strong>
                  <span>{copy.localImage} · {item.runtime}</span>
                </button>
              ))}
            </div>
          )}
          <div className="setup-image-row">
            <input
              value={image}
              onChange={(event) => {
                setImage(event.target.value)
                setEnvironmentId(null)
                setPull(true)
              }}
              placeholder={plan.defaultImage}
              spellCheck={false}
            />
            <label className="setup-check">
              <input type="checkbox" checked={pull} onChange={(event) => setPull(event.target.checked)} />
              {copy.pull}
            </label>
          </div>
          {plan.environments.length > 0 && (
            <>
              <h3>{copy.existingEnv}</h3>
              <div className="setup-options">
                {plan.environments.map((item) => (
                  <button
                    key={`${item.runtime}:${item.id}`}
                    className={`option-card ${image === item.image ? 'selected' : ''}`}
                    onClick={() => {
                      setImage(item.image)
                      setRuntime(item.runtime)
                      setPull(false)
                      setEnvironmentId(item.id)
                    }}
                  >
                    <strong>{item.name}</strong>
                    <span>{item.image} · {item.state}</span>
                  </button>
                ))}
              </div>
            </>
          )}

          <div className="setup-actions">
            <button className="ghost" onClick={() => setStep(0)}>{copy.back}</button>
            <button className="primary" onClick={() => setStep(2)} disabled={!runtime}>{copy.next}</button>
          </div>
        </section>
      )}

      {step === 2 && (
        <section>
          <h3>{copy.agents}</h3>
          <p className="muted">{copy.agentsHint}</p>
          <div className="setup-agent-list">
            {plan.clients.map((client) => (
              <label key={client.id} className="setup-agent">
                <input
                  type="checkbox"
                  checked={clients.includes(client.id)}
                  onChange={(event) =>
                    setClients((previous) =>
                      event.target.checked
                        ? [...previous, client.id]
                        : previous.filter((id) => id !== client.id),
                    )
                  }
                />
                <span>
                  <strong>{client.name}</strong>
                  <em>{statusLabel(language, client.status)}</em>
                  <code>{client.path || '—'}</code>
                </span>
              </label>
            ))}
          </div>
          <div className="setup-actions">
            <button className="ghost" onClick={() => setStep(1)}>{copy.back}</button>
            <button className="primary" onClick={() => void apply()} disabled={busy}>
              {busy ? copy.applying : copy.apply}
            </button>
          </div>
        </section>
      )}

      {step === 3 && result && (
        <section>
          <h3>{copy.done}</h3>
          <p className="muted">
            {result.runner} · {result.image ?? copy.noImage}
          </p>
          {result.configured.length > 0 && (
            <p><strong>{copy.success}:</strong> {result.configured.join(', ')}</p>
          )}
          {result.failed.length > 0 && (
            <div className="error">
              {result.failed.map((failure) => `${failure.id}: ${failure.error}`).join(' · ')}
            </div>
          )}
          <p className="muted">{copy.restart}</p>
          <div className="setup-actions">
            <button className="ghost" onClick={() => { setStep(2); void load() }}>{copy.addMore}</button>
            <button className="primary" onClick={onDone}>{copy.next}</button>
          </div>
        </section>
      )}
    </div>
  )
}
