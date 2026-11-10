import '#components/resizable-columns.css'

import {
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'

type ResizableColumnsProps = {
  first: ReactNode
  second: ReactNode
  storageKey: string | null
  firstSize: number
  firstMinSize: number
  secondMinSize: number
  label: string
}

function readStoredSize(storageKey: string | null, fallback: number) {
  if (storageKey === null) return fallback
  const storedSize = Number(window.localStorage.getItem(storageKey))
  return Number.isFinite(storedSize) && storedSize > 0 ? storedSize : fallback
}

export function ResizableColumns({
  first,
  second,
  storageKey,
  firstSize: initialFirstSize,
  firstMinSize,
  secondMinSize,
  label,
}: ResizableColumnsProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const dragStartRef = useRef<{
    pointerId: number
    x: number
    size: number
  } | null>(null)
  const [containerWidth, setContainerWidth] = useState(0)
  const [firstSize, setFirstSize] = useState(() =>
    readStoredSize(storageKey, initialFirstSize),
  )

  useLayoutEffect(() => {
    const container = containerRef.current
    if (container === null) return

    const fitToContainer = () => {
      const containerWidth = container.getBoundingClientRect().width
      if (containerWidth === 0) return
      setContainerWidth(containerWidth)
      const maxSize = Math.max(firstMinSize, containerWidth - secondMinSize - 8)
      setFirstSize((currentSize) => {
        const nextSize = Math.min(maxSize, Math.max(firstMinSize, currentSize))
        if (nextSize !== currentSize && storageKey !== null) {
          window.localStorage.setItem(storageKey, String(Math.round(nextSize)))
        }
        return nextSize
      })
    }

    const observer = new ResizeObserver(fitToContainer)
    observer.observe(container)
    fitToContainer()
    return () => {
      observer.disconnect()
    }
  }, [firstMinSize, secondMinSize, storageKey])

  const clampSize = (size: number) => {
    const containerWidth =
      containerRef.current?.getBoundingClientRect().width ?? 0
    const maxSize =
      containerWidth > 0
        ? Math.max(firstMinSize, containerWidth - secondMinSize - 8)
        : Number.POSITIVE_INFINITY
    return Math.min(maxSize, Math.max(firstMinSize, size))
  }

  const updateSize = (size: number) => {
    const nextSize = clampSize(size)
    setFirstSize(nextSize)
    if (storageKey !== null) {
      window.localStorage.setItem(storageKey, String(Math.round(nextSize)))
    }
  }

  const maxSize =
    containerWidth === 0
      ? undefined
      : Math.max(firstMinSize, containerWidth - secondMinSize - 8)

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
        className="resizable-columns-pane resizable-columns-first-pane"
        style={{ flexBasis: `${String(firstSize)}px` }}
      >
        {first}
      </div>
      {second === null ? null : (
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
          <div className="resizable-columns-pane resizable-columns-second-pane">
            {second}
          </div>
        </>
      )}
    </div>
  )
}
