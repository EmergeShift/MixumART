import { getCurrentWindow } from '@tauri-apps/api/window'

import { Icon } from './Icons'
import type { ResolvedTheme } from '../theme'

export function ThemeToggle({
  theme,
  onToggle,
}: {
  theme: ResolvedTheme
  onToggle: () => void
}) {
  const dark = theme === 'dark'
  return (
    <button
      type="button"
      className="icon-btn"
      onClick={onToggle}
      title={dark ? '切换到浅色' : '切换到深色'}
      aria-label={dark ? '切换到浅色' : '切换到深色'}
    >
      <Icon name={dark ? 'sun' : 'moon'} size={15} />
    </button>
  )
}

function windowAction(action: 'minimize' | 'maximize' | 'close') {
  try {
    const win = getCurrentWindow()
    if (action === 'minimize') void win.minimize().catch(() => {})
    else if (action === 'maximize') void win.toggleMaximize().catch(() => {})
    else void win.close().catch(() => {})
  } catch {
    // 浏览器预览没有 Tauri IPC
  }
}

export function WindowControls() {
  return (
    <div className="window-controls">
      <button
        type="button"
        className="window-control"
        onClick={() => windowAction('minimize')}
        title="最小化"
        aria-label="最小化"
      >
        <Icon name="minimize" size={14} />
      </button>
      <button
        type="button"
        className="window-control"
        onClick={() => windowAction('maximize')}
        title="最大化"
        aria-label="最大化"
      >
        <Icon name="maximize" size={12} />
      </button>
      <button
        type="button"
        className="window-control close"
        onClick={() => windowAction('close')}
        title="关闭"
        aria-label="关闭"
      >
        <Icon name="close" size={14} />
      </button>
    </div>
  )
}
