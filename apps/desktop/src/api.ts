// 所有 invoke 都收在这一层：组件不直接拼命令名，方便统一改错误处理。
import { invoke } from '@tauri-apps/api/core'

import type { BackendStatus, Container, HostInfo, RuntimeKind } from './types'

export const hostInfo = () => invoke<HostInfo>('host_info')

export const runtimeStatus = () => invoke<BackendStatus[]>('runtime_status')

export const listContainers = (kind: RuntimeKind) =>
  invoke<Container[]>('list_containers', { kind })

export function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return JSON.stringify(err)
}
