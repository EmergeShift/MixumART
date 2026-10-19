import type { RuntimeStatus } from './types'

export type Tone = 'ok' | 'warn' | 'err' | 'muted'

export function statusLabel(status: RuntimeStatus): string {
  switch (status.state) {
    case 'ready':
      return '可用'
    case 'not_installed':
      return '未安装'
    case 'not_running':
      return '未运行'
    case 'unsupported':
      return '不可用'
  }
}

export function statusTone(status: RuntimeStatus): Tone {
  switch (status.state) {
    case 'ready':
      return 'ok'
    case 'not_installed':
      return 'err'
    case 'not_running':
      return 'warn'
    case 'unsupported':
      return 'muted'
  }
}

export function statusDetail(status: RuntimeStatus): string {
  switch (status.state) {
    case 'ready':
      return status.version
        ? `版本 ${status.version}${status.detail ? ` · ${status.detail}` : ''}`
        : '已就绪'
    case 'not_installed':
      return status.hint
    case 'not_running':
      return status.hint
    case 'unsupported':
      return status.reason
  }
}
