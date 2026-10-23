import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { remoteClipboard, surfaceInput, surfacePutFile, surfaceTakeover } from './api'
import type { SurfaceFrame, SurfaceInfo, SurfaceInputEvent } from './types'

type Props = {
  surface: number
  addr: string
}

export function SurfaceView({ surface, addr }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const chainRef = useRef<Promise<void>>(Promise.resolve())
  const [info, setInfo] = useState<SurfaceInfo | null>(null)
  const [status, setStatus] = useState(`连接 ${addr} …`)
  const [takeoverMode, setTakeoverMode] = useState<string | null>(null)
  const [takeoverPending, setTakeoverPending] = useState(false)

  useEffect(() => {
    let disposed = false
    const unlisten = [
      listen<SurfaceInfo>('surface-info', (event) => {
        if (event.payload.id !== surface) return
        setInfo(event.payload)
        const canvas = canvasRef.current
        if (canvas && (canvas.width !== event.payload.width || canvas.height !== event.payload.height)) {
          canvas.width = event.payload.width
          canvas.height = event.payload.height
        }
        setStatus('')
      }),
      listen<SurfaceFrame>('surface-frame', (event) => {
        const frame = event.payload
        if (frame.surface !== surface) return
        chainRef.current = chainRef.current
          .then(async () => {
            if (disposed) return
            const canvas = canvasRef.current
            if (!canvas) return
            const full =
              frame.x === 0 && frame.y === 0 && frame.w === frame.fullW && frame.h === frame.fullH
            if (full) {
              if (canvas.width !== frame.fullW || canvas.height !== frame.fullH) {
                canvas.width = frame.fullW
                canvas.height = frame.fullH
              }
            } else if (canvas.width === 0 || canvas.height === 0) {
              canvas.width = frame.fullW
              canvas.height = frame.fullH
            }
            const ctx = canvas.getContext('2d')
            if (!ctx) return
            if (full) ctx.clearRect(0, 0, canvas.width, canvas.height)
            const bitmap = await createImageBitmap(
              new Blob([new Uint8Array(frame.data)], { type: 'image/png' }),
            )
            ctx.drawImage(bitmap, frame.x, frame.y, frame.w, frame.h)
            bitmap.close()
            setStatus('')
          })
          .catch((error: unknown) => setStatus(String(error)))
      }),
      listen<{ surface: number }>('surface-destroyed', () => {
        void getCurrentWindow().close()
      }),
      listen<string>('surface-disconnected', (event) => {
        setStatus(`显示连接断开：${event.payload}`)
      }),
    ]

    return () => {
      disposed = true
      for (const pending of unlisten) void pending.then((fn) => fn())
    }
  }, [addr, surface])

  const send = (event: SurfaceInputEvent) => {
    void surfaceInput(surface, event).catch((error: unknown) => setStatus(String(error)))
  }

  // 滚轮/触控板：React 的 onWheel 在根节点上是 passive 的，拦不住页面滚动，
  // 这里挂原生非 passive 监听。触控板一次只有几像素，按 40px = 1 格累积，
  // 每帧最多发 3 格——否则一次甩动会把输入队列灌满（issue #47 的卡死）。
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let accumulated = 0
    let queued = 0
    let frame = 0
    const flush = () => {
      frame = 0
      if (queued === 0) return
      const action = queued > 0 ? 'scroll_down' : 'scroll_up'
      const steps = Math.min(Math.abs(queued), 3)
      queued += queued > 0 ? -steps : steps
      for (let i = 0; i < steps; i += 1) send({ type: 'pointer_axis', surface, action })
      if (queued !== 0) frame = window.requestAnimationFrame(flush)
    }
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1
      accumulated += event.deltaY * unit
      const notches = Math.trunc(accumulated / 40)
      if (notches === 0) return
      accumulated -= notches * 40
      queued += notches
      if (frame === 0) frame = window.requestAnimationFrame(flush)
    }
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      canvas.removeEventListener('wheel', onWheel)
      if (frame !== 0) window.cancelAnimationFrame(frame)
    }
  }, [surface])

  const point = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const canvas = event.currentTarget
    const rect = canvas.getBoundingClientRect()
    return {
      x: ((event.clientX - rect.left) * canvas.width) / rect.width,
      y: ((event.clientY - rect.top) * canvas.height) / rect.height,
    }
  }

  const button = (code: number): 'left' | 'right' | 'middle' =>
    code === 2 ? 'right' : code === 1 ? 'middle' : 'left'

  // Map a text character to the keysyms the compositor's evdev table knows.
  // Non-ASCII (CJK IME) is reported instead of silently typing the wrong key.
  const charName = (ch: string): string | null => {
    if (ch === '\n' || ch === '\r') return 'Return'
    if (ch === ' ') return 'space'
    if (/^[a-z0-9]$/.test(ch)) return ch
    const punctuation: Record<string, string> = {
      '-': 'minus',
      '=': 'equal',
      ',': 'comma',
      '.': 'period',
      '/': 'slash',
      ';': 'semicolon',
      "'": 'apostrophe',
    }
    return punctuation[ch] ?? null
  }

  const sendText = (text: string) => {
    // Text-input clients (GTK/Qt/…) accept composed text including CJK.
    // Keep the existing key path for pure ASCII so terminals stay compatible.
    if (/[^\x00-\x7F]/.test(text)) {
      send({ type: 'text', surface, text })
      return
    }
    for (const ch of text) {
      const keysym = charName(ch)
      if (!keysym) continue
      send({ type: 'key', surface, keysym, action: 'press' })
      send({ type: 'key', surface, keysym, action: 'release' })
    }
  }

  const keyName = (event: React.KeyboardEvent): string | null => {
    if (event.key === ' ') return 'space'
    if (event.key.length === 1) return event.key.toLowerCase()
    switch (event.key) {
      case 'Enter':
        return 'Return'
      case 'Backspace':
        return 'BackSpace'
      case ' ':
        return 'space'
      case 'Tab':
      case 'Escape':
      case 'ArrowLeft':
      case 'ArrowRight':
      case 'ArrowUp':
      case 'ArrowDown':
        return event.key.replace('Arrow', '')
      default:
        return null
    }
  }

  const toggleTakeover = async () => {
    const action = takeoverMode === 'user' ? 'release' : 'claim'
    setTakeoverPending(true)
    try {
      const mode = await surfaceTakeover(surface, action)
      setTakeoverMode(mode)
      setStatus(mode === 'user' ? '已接管窗口' : '已释放窗口')
    } catch (error: unknown) {
      setStatus(String(error))
    } finally {
      setTakeoverPending(false)
    }
  }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === 't') {
        event.preventDefault()
        void toggleTakeover()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [takeoverMode, takeoverPending])

  return (
    <div className="surface-view">
      <div className="surface-toolbar">
        <button
          type="button"
          className="ghost"
          onClick={() => void toggleTakeover()}
          disabled={takeoverPending}
          title="快捷键：Ctrl/Cmd+Shift+T"
        >
          {takeoverMode === 'user' ? '释放窗口' : '接管窗口'}
        </button>
      </div>
      <canvas
        ref={canvasRef}
        className="surface-canvas"
        tabIndex={0}
        onPointerMove={(event) => send({ type: 'pointer_motion', surface, ...point(event) })}
        onPointerDown={(event) => {
          event.currentTarget.focus()
          event.currentTarget.setPointerCapture(event.pointerId)
          send({
            type: 'pointer_button',
            surface,
            button: button(event.button),
            action: 'press',
          })
        }}
        onPointerUp={(event) => {
          event.currentTarget.releasePointerCapture(event.pointerId)
          send({
            type: 'pointer_button',
            surface,
            button: button(event.button),
            action: 'release',
          })
        }}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.altKey && event.key.toLowerCase() === 'v') {
            event.preventDefault()
            void remoteClipboard(surface)
              .then((text) => navigator.clipboard.writeText(text))
              .catch((error: unknown) => setStatus(String(error)))
            return
          }
          if (event.nativeEvent.isComposing) return
          const keysym = keyName(event)
          if (!keysym) return
          event.preventDefault()
          send({ type: 'key', surface, keysym, action: 'press' })
        }}
        onKeyUp={(event) => {
          if (event.nativeEvent.isComposing) return
          const keysym = keyName(event)
          if (!keysym) return
          event.preventDefault()
          send({ type: 'key', surface, keysym, action: 'release' })
        }}
        onPaste={(event) => {
          const text = event.clipboardData.getData('text')
          if (!text) return
          event.preventDefault()
          sendText(text)
        }}
        onCompositionEnd={(event) => {
          if (event.data) sendText(event.data)
        }}
        onDragOver={(event) => {
          event.preventDefault()
          if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
        }}
        onDrop={(event) => {
          event.preventDefault()
          const text =
            event.dataTransfer.getData('text/plain') ||
            event.dataTransfer.getData('text/uri-list')
          if (text) {
            send({ type: 'text', surface, text })
            return
          }
          const files = Array.from(event.dataTransfer.files ?? [])
          if (files.length === 0) return
          void (async () => {
            const paths: string[] = []
            for (const file of files) {
              const data = Array.from(new Uint8Array(await file.arrayBuffer()))
              paths.push(await surfacePutFile(surface, file.name, data))
            }
            send({ type: 'text', surface, text: paths.join(' ') })
          })().catch((error: unknown) => setStatus(String(error)))
        }}
      />
      {status && <div className="surface-status">{status}</div>}
      {info && <div className="surface-meta">{info.title || `surface ${info.id}`}</div>}
    </div>
  )
}
