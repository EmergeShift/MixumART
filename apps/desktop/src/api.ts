// 所有 invoke 都收在这一层：组件不直接拼命令名，方便统一改错误处理。
// 浏览器（无 Tauri）下返回演示数据，让设计评审可以直接跑 `npm run dev`。
import { invoke } from '@tauri-apps/api/core'

import { demoBackends, demoContainers, demoHost, demoSurfaces, isDemo } from './demo'
import type {
  BackendStatus,
  Container,
  HostInfo,
  RuntimeKind,
  SurfaceInfo,
  SurfaceInputEvent,
} from './types'

const demoDelay = <T,>(value: T): Promise<T> =>
  new Promise((resolve) => setTimeout(() => resolve(value), 120))

export const hostInfo = () =>
  isDemo() ? demoDelay(demoHost) : invoke<HostInfo>('host_info')

export const runtimeStatus = () =>
  isDemo() ? demoDelay(demoBackends) : invoke<BackendStatus[]>('runtime_status')

export const listContainers = (kind: RuntimeKind) =>
  isDemo()
    ? demoDelay(kind === 'apple_container' ? demoContainers : [])
    : invoke<Container[]>('list_containers', { kind })

export function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return JSON.stringify(err)
}

export const listSurfaces = (addr: string, token: string, fingerprint?: string) =>
  isDemo()
    ? demoDelay(demoSurfaces)
    : invoke<SurfaceInfo[]>('list_surfaces', {
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
  isDemo()
    ? Promise.resolve()
    : invoke<void>('open_surface_window', {
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
  isDemo() ? Promise.resolve() : invoke<void>('surface_input', { surface, event })

export const remoteClipboard = (surface: number) =>
  isDemo() ? Promise.resolve('clipboard unavailable in demo') : invoke<string>('remote_clipboard', { surface })

export const surfacePutFile = (surface: number, name: string, data: number[]) =>
  isDemo()
    ? Promise.resolve(`/demo/transfers/${name}`)
    : invoke<string>('surface_put_file', { surface, name, data })
