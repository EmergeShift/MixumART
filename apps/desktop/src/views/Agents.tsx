import { useState } from 'react'

import type { AgentMode } from '../types'

const MODES: { key: AgentMode; title: string; summary: string; detail: string }[] = [
  {
    key: 'as-tool',
    title: '作为工具',
    summary: '外部 Agent 直接使用环境',
    detail:
      '例如 Codex 把 MART 当作它的执行环境：通过 MAP 提交任务、执行命令、观看窗口。容器内的 Agent Core 不运行。',
  },
  {
    key: 'sub-agent',
    title: 'Sub-agent',
    summary: '外部 Agent 把活派给 MART',
    detail:
      'Codex 作为编排者派发任务，容器内的 Agent Core 执行工作。模型连接由用户自带（BYOK），凭证不进入快照。',
  },
  {
    key: 'isolated',
    title: '隔离环境',
    summary: '一个 Agent 一个容器',
    detail:
      '把指定平台的凭证迁移进特定容器，会话与记忆只属于那个环境。适合把互不信任的 Agent 彻底隔开。',
  },
]

const CORES = [
  { key: 'mixum', name: 'Mixum Core', note: '内置，与快照/接管深度集成' },
  { key: 'pi', name: 'Pi', note: '接入 Pi 的 harness' },
  { key: 'deepseek', name: 'DeepSeek Harness', note: '接入 DeepSeek 的 agent 框架' },
  { key: 'codex', name: 'Codex', note: '以 Codex 作为容器内核心' },
]

const PLATFORMS = [
  { name: 'Codex', kind: 'CLI agent' },
  { name: 'Claude Code', kind: 'CLI agent' },
  { name: 'Cursor', kind: 'IDE / GUI' },
  { name: '自定义（MAP 客户端）', kind: '任意实现' },
]

export function Agents() {
  const [core, setCore] = useState('mixum')

  return (
    <div className="agents-page">
      <section>
        <div className="section-head">
          <div>
            <h2>Agent 工作模式</h2>
            <p className="muted">
              MART 不运行 Agent 本体；它决定外部 Agent 与容器内 Agent Core 如何分工。
            </p>
          </div>
        </div>
        <div className="mode-grid">
          {MODES.map((mode) => (
            <article className="mode-card" key={mode.key}>
              <span className="tag">{mode.title}</span>
              <h3>{mode.summary}</h3>
              <p className="muted">{mode.detail}</p>
              <span className="pending">配置接入中</span>
            </article>
          ))}
        </div>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>Agent Core</h2>
            <p className="muted">容器内执行工作的核心。用户可选择我们的，也可以用别人的 harness。</p>
          </div>
        </div>
        <div className="core-grid">
          {CORES.map((item) => (
            <button
              key={item.key}
              className={core === item.key ? 'core-card active' : 'core-card'}
              onClick={() => setCore(item.key)}
            >
              <strong>{item.name}</strong>
              <span className="muted">{item.note}</span>
              <span className="radio" />
            </button>
          ))}
        </div>
        <p className="muted footnote">选择持久化与启动装配随后端接入（当前仅界面状态）。</p>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>平台接入</h2>
            <p className="muted">已接入或计划接入的 Agent 平台；连接后为其分配环境与工作模式。</p>
          </div>
          <button className="ghost" disabled title="连接器接入后可用">
            添加平台
          </button>
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>平台</th>
              <th>类型</th>
              <th>状态</th>
              <th>默认模式</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {PLATFORMS.map((platform) => (
              <tr key={platform.name}>
                <td>{platform.name}</td>
                <td className="muted">{platform.kind}</td>
                <td>
                  <span className="state state-stopped">
                    <span className="dot" />
                    未连接
                  </span>
                </td>
                <td className="muted">—</td>
                <td className="row-actions">
                  <button className="ghost" disabled title="连接器接入后可用">
                    配置
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>凭证与 BYOK</h2>
            <p className="muted">
              平台凭证由宿主保管，任务开始时注入目标容器；凭证永不进入快照（见 TRUST.md）。
            </p>
          </div>
        </div>
        <div className="callout">
          <div>
            <strong>凭证迁移到容器</strong>
            <p className="muted">
              vault.* 接入后开放：选择平台 → 选择容器 → 注入并立即擦除宿主侧副本；
              快照中只保留会话状态，不含密钥。
            </p>
          </div>
          <button className="ghost" disabled title="vault.* 接入后可用">
            迁移凭证
          </button>
        </div>
      </section>
    </div>
  )
}
