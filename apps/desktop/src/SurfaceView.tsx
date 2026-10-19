import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { remoteClipboard, surfaceInput } from './api'
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
    let skipped = false
    for (const ch of text) {
      const keysym = charName(ch)
      if (!keysym) {
        skipped = true
        continue
      }
      send({ type: 'key', surface, keysym, action: 'press' })
      send({ type: 'key', surface, keysym, action: 'release' })
    }
    if (skipped) setStatus('部分字符不是 ASCII，当前 B 输入通路仅支持 ASCII')
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

  return (
    <div className="surface-view">
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
      />
      {status && <div className="surface-status">{status}</div>}
      {info && <div className="surface-meta">{info.title || `surface ${info.id}`}</div>}
    </div>
  )
}
