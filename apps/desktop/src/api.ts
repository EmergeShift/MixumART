// 所有 invoke 都收在这一层：组件不直接拼命令名，方便统一改错误处理。
import { invoke } from '@tauri-apps/api/core'

import type {
  BackendStatus,
  Container,
  HostInfo,
  RuntimeKind,
  SurfaceInfo,
  SurfaceInputEvent,
} from './types'

export const hostInfo = () => invoke<HostInfo>('host_info')

export const runtimeStatus = () => invoke<BackendStatus[]>('runtime_status')

export const listContainers = (kind: RuntimeKind) =>
  invoke<Container[]>('list_containers', { kind })

export function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return JSON.stringify(err)
}

export const listSurfaces = (addr: string, token: string, fingerprint?: string) =>
  invoke<SurfaceInfo[]>('list_surfaces', {
    addr,
    token,
    fingerprint: fingerprint || null,
  })

export const openSurfaceWindow = (
  addr: string,
  token: string,
  fingerprint: string | undefined,
  surface: SurfaceInfo,
) =>
  invoke<void>('open_surface_window', {
    request: {
      addr,
      token,
      fingerprint: fingerprint || null,
      surface: surface.id,
      title: surface.title,
      width: surface.width,
      height: surface.height,
    },
  })

export const surfaceInput = (surface: number, event: SurfaceInputEvent) =>
  invoke<void>('surface_input', { surface, event })

export const remoteClipboard = (surface: number) =>
  invoke<string>('remote_clipboard', { surface })

export const surfacePutFile = (surface: number, name: string, data: number[]) =>
  invoke<string>('surface_put_file', { surface, name, data })
