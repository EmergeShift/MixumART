import { useState } from 'react'

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

const PLATFORMS = [
  { name: 'Codex', kind: '命令行 Agent' },
  { name: 'Claude Code', kind: '命令行 Agent' },
  { name: 'Cursor', kind: '编辑器' },
  { name: '其它 Agent', kind: '支持 MAP 协议即可' },
]

export function Agents() {
  const [core, setCore] = useState('mixum')

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
              <span className="pending">「{mode.title}」的配置功能即将开放</span>
            </article>
          ))}
        </div>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>环境里由谁来干活</h2>
            <p>默认使用内置助手；也可以换成你习惯的 Agent 引擎。</p>
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
        <p className="footnote">当前只是界面选择；保存与生效会随环境创建功能一起开放。</p>
      </section>

      <section>
        <div className="section-head">
          <div>
            <h2>已接入与计划接入的 Agent</h2>
            <p>连接后，可以为每个 Agent 指定使用哪个环境、用哪种分工方式。</p>
          </div>
          <button className="ghost" disabled title="连接功能即将开放">
            添加 Agent
          </button>
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>Agent</th>
              <th>类型</th>
              <th>状态</th>
              <th>默认使用方式</th>
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
                  <button className="ghost" disabled title="连接功能即将开放">
                    连接
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
            <h2>登录凭据</h2>
            <p>Agent 的账号钥匙由这台电脑保管，只交给指定的环境，不写进快照。</p>
          </div>
        </div>
        <div className="callout">
          <div>
            <strong>把凭据放进指定环境</strong>
            <p>
              选择 Agent 和环境后，凭据会在开始时临时交给环境，结束后立即清除。
              环境生成的快照里只有工作内容，没有密钥。
            </p>
          </div>
          <button className="ghost" disabled title="凭据管理即将开放">
            管理凭据
          </button>
        </div>
      </section>
    </div>
  )
}
