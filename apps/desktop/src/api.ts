// 所有 invoke 都收在这一层：组件不直接拼命令名，方便统一改错误处理。
// 浏览器（无 Tauri）下返回演示数据，让设计评审可以直接跑 `npm run dev`。
import { invoke } from '@tauri-apps/api/core'

import { demoBackends, demoContainers, demoHost, demoSurfaces, isDemo } from './demo'
import type {
  BackendStatus,
  ConnectionInfo,
  Container,
  CreateSpec,
  EnvironmentConnection,
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

export type CommandResult = {
  taskId: string
  state: string
  lines: string[]
  error?: string | null
}

export const openContainerTerminal = (
  kind: RuntimeKind,
  containerId: string,
  cols = 100,
  rows = 30,
) =>
  isDemo()
    ? demoDelay(1)
    : invoke<number>('open_container_terminal', { kind, containerId, cols, rows })

export const containerTerminalInput = (terminal: number, data: string) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('container_terminal_input', { terminal, data })

export const containerTerminalResize = (terminal: number, cols: number, rows: number) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('container_terminal_resize', { terminal, cols, rows })

export const closeContainerTerminal = (terminal: number) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('close_container_terminal', { terminal })

export const runEnvironmentCommand = (containerId: string, command: string) =>
  isDemo()
    ? demoDelay<CommandResult>({ taskId: 'demo', state: 'completed', lines: [`$ ${command}`, '(demo)'], error: null })
    : invoke<CommandResult>('environment_run_command', { containerId, command })

export const snapshotTree = (containerId: string) =>
  isDemo()
    ? demoDelay({ head: 'demo', branch: 'main', nodes: [] })
    : invoke<unknown>('environment_snapshot_tree', { containerId })

export const snapshotCreate = (containerId: string, label: string) =>
  isDemo()
    ? demoDelay({ id: 'demo', label })
    : invoke<unknown>('environment_snapshot_create', { containerId, label: label || null })

export const snapshotRestore = (containerId: string, node: string) =>
  isDemo()
    ? demoDelay({ restoredTo: node })
    : invoke<unknown>('environment_snapshot_restore', { containerId, node })

export const snapshotFork = (containerId: string, node: string, branch: string) =>
  isDemo()
    ? demoDelay({ branch })
    : invoke<unknown>('environment_snapshot_fork', { containerId, node, branch })

export type AgentProfile = {
  id: string
  name: string
  kind: string
  mode: string
  core: string
  command: string
  environmentId?: string
}

export const setAgentCredential = (agentId: string, secret: string) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('set_agent_credential', { agentId, secret })

export const deleteAgentCredential = (agentId: string) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('delete_agent_credential', { agentId })

export const hasAgentCredential = (agentId: string) =>
  isDemo() ? demoDelay(false) : invoke<boolean>('has_agent_credential', { agentId })

export const loadAgentProfiles = () =>
  isDemo() ? demoDelay<AgentProfile[]>([]) : invoke<AgentProfile[]>('load_agent_profiles')

export const saveAgentProfile = (profile: AgentProfile) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('save_agent_profile', { profile })

export const removeAgentProfile = (id: string) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('remove_agent_profile', { id })

export const runtimeImages = (kind: RuntimeKind) =>
  isDemo()
    ? demoDelay(['mixumos:0.1.4'])
    : invoke<string[]>('runtime_images', { kind })

export const runtimePullImage = (kind: RuntimeKind, reference: string) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('runtime_pull_image', { kind, reference })

export const runtimeLoadImage = (kind: RuntimeKind, path: string) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('runtime_load_image', { kind, path })

export const runtimePrune = (kind: RuntimeKind) =>
  isDemo()
    ? demoDelay('demo prune ok')
    : invoke<string>('runtime_prune', { kind })

export const runtimeLogs = (kind: RuntimeKind, id: string, tail = 200) =>
  isDemo()
    ? demoDelay(['demo log line'])
    : invoke<string[]>('runtime_logs', { kind, id, tail })

export const runtimeDiskUsage = (kind: RuntimeKind) =>
  isDemo()
    ? demoDelay('Images 1.2GB · Containers 3.4GB · Volumes 0B')
    : invoke<string>('runtime_disk_usage', { kind })

export const startEnvironment = (kind: RuntimeKind, id: string) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('start_environment', { kind, id })

export const stopEnvironment = (kind: RuntimeKind, id: string) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('stop_environment', { kind, id })

export const removeEnvironment = (kind: RuntimeKind, id: string) =>
  isDemo() ? demoDelay(undefined) : invoke<void>('remove_environment', { kind, id })

export const createEnvironment = (kind: RuntimeKind, spec: CreateSpec) =>
  isDemo()
    ? demoDelay(demoContainers[0])
    : invoke<Container>('create_environment', { kind, spec })

export const connectEnvironment = (
  kind: RuntimeKind,
  containerId: string,
  addr: string,
) =>
  isDemo()
    ? demoDelay<ConnectionInfo>({ addr, fingerprint: null })
    : invoke<ConnectionInfo>('connect_environment', { kind, containerId, addr })

export const listEnvironmentSurfaces = (containerId: string) =>
  isDemo()
    ? demoDelay(demoSurfaces)
    : invoke<SurfaceInfo[]>('list_environment_surfaces', { containerId })

export const openEnvironmentSurface = (
  containerId: string,
  surface: SurfaceInfo,
) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('open_environment_surface', {
        containerId,
        surface: surface.id,
        title: surface.title,
        width: surface.width,
        height: surface.height,
      })

export const loadEnvironmentConnections = () =>
  isDemo()
    ? demoDelay<Record<string, ConnectionInfo>>({})
    : invoke<Record<string, ConnectionInfo>>('load_environment_connections')

export const saveEnvironmentConnection = (
  containerId: string,
  connection: EnvironmentConnection,
) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('save_environment_connection', { containerId, connection })

export const removeEnvironmentConnection = (containerId: string) =>
  isDemo()
    ? demoDelay(undefined)
    : invoke<void>('remove_environment_connection', { containerId })

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

export const surfaceTakeover = (surface: number, action: 'claim' | 'release') =>
  isDemo()
    ? demoDelay(action === 'claim' ? 'user' : 'free')
    : invoke<string>('surface_takeover', { surface, action })

export const remoteClipboard = (surface: number) =>
  isDemo() ? Promise.resolve('clipboard unavailable in demo') : invoke<string>('remote_clipboard', { surface })

export const surfacePutFile = (surface: number, name: string, data: number[]) =>
  isDemo()
    ? Promise.resolve(`/demo/transfers/${name}`)
    : invoke<string>('surface_put_file', { surface, name, data })
