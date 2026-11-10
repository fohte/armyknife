import '#components/resizable-columns.css'

import { Result } from 'neverthrow'
import {
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'

const separatorSize = 8

type ResizableColumnsProps = {
  first: ReactNode
  second: ReactNode
  storageKey: string | null
  firstSize: number
  firstMinSize: number
  secondMinSize: number
  label: string
  fillFirstPaneWhenAlone?: boolean
}

function readStoredSize(storageKey: string | null, fallback: number) {
  if (storageKey === null) return fallback
  const storedValue = Result.fromThrowable(
    () => window.localStorage.getItem(storageKey),
    () => null,
  )()
  if (storedValue.isErr() || storedValue.value === null) return fallback
  const storedSize = Number(storedValue.value)
  return Number.isFinite(storedSize) && storedSize > 0 ? storedSize : fallback
}

function saveStoredSize(storageKey: string | null, size: number) {
  if (storageKey === null) return
  Result.fromThrowable(
    () => {
      window.localStorage.setItem(storageKey, String(Math.round(size)))
    },
    () => undefined,
  )().match(
    () => undefined,
    () => undefined,
  )
}

function getMaximumFirstSize(
  containerWidth: number,
  firstMinSize: number,
  secondMinSize: number,
) {
  if (containerWidth === 0) return undefined
  return Math.max(firstMinSize, containerWidth - secondMinSize - separatorSize)
}

export function ResizableColumns({
  first,
  second,
  storageKey,
  firstSize: initialFirstSize,
  firstMinSize,
  secondMinSize,
  label,
  fillFirstPaneWhenAlone = true,
}: ResizableColumnsProps) {
  const hasSecond = second !== null
  const containerRef = useRef<HTMLDivElement>(null)
  const secondPaneRef = useRef<HTMLDivElement>(null)
  const dragStartRef = useRef<{
    pointerId: number
    x: number
    size: number
  } | null>(null)
  const [containerWidth, setContainerWidth] = useState(0)
  const [secondPaneVisible, setSecondPaneVisible] = useState(hasSecond)
  const [preferredFirstSize, setPreferredFirstSize] = useState(() =>
    readStoredSize(storageKey, initialFirstSize),
  )
  const maxSize = secondPaneVisible
    ? getMaximumFirstSize(containerWidth, firstMinSize, secondMinSize)
    : undefined
  const firstSize = Math.min(
    Math.max(firstMinSize, preferredFirstSize),
    maxSize ?? Number.POSITIVE_INFINITY,
  )

  useLayoutEffect(() => {
    const container = containerRef.current
    if (container === null) return

    const fitToContainer = () => {
      const containerWidth = container.getBoundingClientRect().width
      if (containerWidth === 0) return
      const secondPaneVisible =
        hasSecond &&
        secondPaneRef.current !== null &&
        window.getComputedStyle(secondPaneRef.current).display !== 'none'
      setSecondPaneVisible(secondPaneVisible)
      setContainerWidth(containerWidth)
    }

    const observer = new ResizeObserver(fitToContainer)
    observer.observe(container)
    if (secondPaneRef.current !== null) {
      observer.observe(secondPaneRef.current)
    }
    fitToContainer()
    return () => {
      observer.disconnect()
    }
  }, [firstMinSize, hasSecond, secondMinSize])

  const clampSize = (size: number) => {
    const containerWidth =
      containerRef.current?.getBoundingClientRect().width ?? 0
    const maxSize = secondPaneVisible
      ? (getMaximumFirstSize(containerWidth, firstMinSize, secondMinSize) ??
        Number.POSITIVE_INFINITY)
      : Number.POSITIVE_INFINITY
    return Math.min(maxSize, Math.max(firstMinSize, size))
  }

  const updateSize = (size: number) => {
    const nextSize = clampSize(size)
    if (nextSize === firstSize) return
    setPreferredFirstSize(nextSize)
    saveStoredSize(storageKey, nextSize)
  }

  const onSeparatorKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 40 : 10
    if (event.key === 'ArrowLeft') {
      event.preventDefault()
      updateSize(firstSize - step)
    } else if (event.key === 'ArrowRight') {
      event.preventDefault()
      updateSize(firstSize + step)
    } else if (event.key === 'Home') {
      event.preventDefault()
      updateSize(firstMinSize)
    } else if (event.key === 'End') {
      event.preventDefault()
      updateSize(maxSize ?? Number.POSITIVE_INFINITY)
    }
  }

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    dragStartRef.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      size: firstSize,
    }
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const dragStart = dragStartRef.current
    if (dragStart?.pointerId === event.pointerId) {
      updateSize(dragStart.size + event.clientX - dragStart.x)
    }
  }

  const stopPointerDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (dragStartRef.current?.pointerId === event.pointerId) {
      dragStartRef.current = null
    }
  }

  return (
    <div className="resizable-columns" ref={containerRef}>
      <div
        className={`resizable-columns-pane resizable-columns-first-pane${!hasSecond && fillFirstPaneWhenAlone ? ' is-alone' : ''}`}
        style={{ flexBasis: `${String(firstSize)}px` }}
      >
        {first}
      </div>
      {!hasSecond ? null : (
        <>
          <div
            aria-label={label}
            aria-orientation="vertical"
            aria-valuemax={
              maxSize === undefined ? undefined : Math.round(maxSize)
            }
            aria-valuemin={firstMinSize}
            aria-valuenow={Math.round(firstSize)}
            className="resizable-columns-separator"
            onKeyDown={onSeparatorKeyDown}
            onPointerCancel={stopPointerDrag}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={stopPointerDrag}
            role="separator"
            tabIndex={0}
          />
          <div
            className="resizable-columns-pane resizable-columns-second-pane"
            ref={secondPaneRef}
          >
            {second}
          </div>
        </>
      )}
    </div>
  )
}

export function renderConversationCritColumns(
  conversation: ReactNode,
  crit: ReactNode,
) {
  return (
    <ResizableColumns
      first={conversation}
      second={crit}
      storageKey="agent-watch:conversation-width"
      firstSize={560}
      firstMinSize={240}
      secondMinSize={300}
      label="Resize conversation and crit"
    />
  )
}
