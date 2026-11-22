import { Button } from '@fohte/ui/button'
import { Clock3, X } from 'lucide-react'
import {
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useRef,
  useState,
} from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'
import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

export function InboxSessionListItem({
  session,
  selectedId,
  onSelect,
  isMobile,
  onLater,
  onCloseSession,
  onOpenActions,
  defaultSwiped = false,
}: {
  session: Session
  selectedId: string | null
  onSelect: (id: string) => void
  isMobile: boolean
  onLater?: ((id: string) => void) | undefined
  onCloseSession?: ((id: string) => void) | undefined
  onOpenActions?: ((session: Session) => void) | undefined
  defaultSwiped?: boolean
}) {
  const showLaterAction = onLater !== undefined
  const showCloseAction =
    onCloseSession !== undefined && session.category !== 'done'
  const swipeWidth = (showLaterAction ? 88 : 0) + (showCloseAction ? 88 : 0)
  const [swiped, setSwiped] = useState(defaultSwiped)
  const [dragOffset, setDragOffset] = useState(defaultSwiped ? -swipeWidth : 0)
  const pointerStart = useRef<{
    pointerId: number
    x: number
    y: number
    baseX: number
  } | null>(null)
  const longPressTimer = useRef<{
    pointerId: number
    timeout: ReturnType<typeof setTimeout>
  } | null>(null)
  const suppressClick = useRef(false)
  const feedback = useOptionalAsyncActionFeedback()
  const closeAction = feedback?.actions.find(
    (action) => action.kind === 'close' && action.sessionId === session.id,
  )
  const canShowActions =
    isMobile &&
    (onLater !== undefined ||
      onCloseSession !== undefined ||
      onOpenActions !== undefined)
  const swipeActionsVisible = swiped || dragOffset < 0

  useEffect(
    () => () => {
      if (longPressTimer.current !== null) {
        clearTimeout(longPressTimer.current.timeout)
      }
    },
    [],
  )

  function clearLongPress(pointerId?: number) {
    const timer = longPressTimer.current
    if (
      timer === null ||
      (pointerId !== undefined && timer.pointerId !== pointerId)
    ) {
      return
    }
    clearTimeout(timer.timeout)
    longPressTimer.current = null
  }

  function handlePointerDown(event: ReactPointerEvent<HTMLButtonElement>) {
    if (!canShowActions || event.button !== 0) return
    const activePointer = pointerStart.current
    if (activePointer !== null && activePointer.pointerId !== event.pointerId) {
      clearLongPress(activePointer.pointerId)
      pointerStart.current = null
      suppressClick.current = true
      setDragOffset(swiped ? -swipeWidth : 0)
      return
    }
    clearLongPress()
    suppressClick.current = false
    pointerStart.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      baseX: swiped ? -swipeWidth : 0,
    }
    setDragOffset(swiped ? -swipeWidth : 0)
    if (event.pointerType === 'touch' && onOpenActions !== undefined) {
      const pointerId = event.pointerId
      const timeout = setTimeout(() => {
        if (
          pointerStart.current?.pointerId !== pointerId ||
          longPressTimer.current?.pointerId !== pointerId
        ) {
          return
        }
        longPressTimer.current = null
        suppressClick.current = true
        pointerStart.current = null
        setSwiped(false)
        setDragOffset(0)
        onOpenActions(session)
      }, 500)
      longPressTimer.current = { pointerId, timeout }
    }
  }

  function handlePointerMove(event: ReactPointerEvent<HTMLButtonElement>) {
    const start = pointerStart.current
    if (start === null || start.pointerId !== event.pointerId) return
    const deltaX = event.clientX - start.x
    const deltaY = event.clientY - start.y
    if (Math.abs(deltaY) > 10 && Math.abs(deltaY) > Math.abs(deltaX)) {
      clearLongPress(event.pointerId)
      pointerStart.current = null
      suppressClick.current = true
      setDragOffset(start.baseX)
      return
    }
    if (Math.abs(deltaX) > 8) clearLongPress(event.pointerId)
    if (Math.abs(deltaX) > Math.abs(deltaY)) {
      setDragOffset(Math.max(-swipeWidth, Math.min(0, start.baseX + deltaX)))
    }
  }

  function handlePointerUp(event: ReactPointerEvent<HTMLButtonElement>) {
    const start = pointerStart.current
    if (start === null || start.pointerId !== event.pointerId) return
    clearLongPress(event.pointerId)
    pointerStart.current = null
    const deltaX = event.clientX - start.x
    if (Math.abs(deltaX) >= 35) {
      suppressClick.current = true
      const nextSwiped = deltaX < -55 || (swiped && deltaX < 35)
      setSwiped(nextSwiped)
      setDragOffset(nextSwiped ? -swipeWidth : 0)
      return
    }
    setDragOffset(swiped ? -swipeWidth : 0)
  }

  return (
    <li className={`session-list-item${swiped ? ' is-swiped' : ''}`}>
      {canShowActions ? (
        <div className="inbox-row-swipe-actions" aria-hidden={!swiped}>
          {!swipeActionsVisible || !showLaterAction ? null : (
            <Button
              type="button"
              variant="plain"
              aria-label={`Later: ${session.title}`}
              tabIndex={swiped ? 0 : -1}
              onClick={() => {
                setSwiped(false)
                setDragOffset(0)
                onLater(session.id)
              }}
            >
              <Clock3 aria-hidden="true" size={18} />
              Later
            </Button>
          )}
          {!swipeActionsVisible || !showCloseAction ? null : (
            <Button
              type="button"
              variant="plain"
              aria-label={`Close: ${session.title}`}
              tabIndex={swiped ? 0 : -1}
              onClick={() => {
                setSwiped(false)
                setDragOffset(0)
                onCloseSession(session.id)
              }}
            >
              <X aria-hidden="true" size={18} />
              Close
            </Button>
          )}
        </div>
      ) : null}
      <Button
        type="button"
        variant="plain"
        className={`session-row cursor-pointer category-${session.category}${session.id === selectedId ? ' is-selected' : ''}`}
        data-session-row=""
        onContextMenu={(event) => {
          if (!canShowActions || onOpenActions === undefined) return
          event.preventDefault()
          clearLongPress()
          pointerStart.current = null
          suppressClick.current = true
          onOpenActions(session)
        }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={(event) => {
          const start = pointerStart.current
          if (start === null || start.pointerId !== event.pointerId) return
          clearLongPress(event.pointerId)
          pointerStart.current = null
          setDragOffset(swiped ? -swipeWidth : 0)
        }}
        style={
          canShowActions
            ? { transform: `translateX(${String(dragOffset)}px)` }
            : undefined
        }
        aria-current={session.id === selectedId ? 'true' : undefined}
        onClick={() => {
          if (suppressClick.current) {
            suppressClick.current = false
            return
          }
          onSelect(session.id)
        }}
      >
        <SessionRow session={session} density="full" isMobile={isMobile} />
      </Button>
      {closeAction === undefined ? null : (
        <AsyncActionItem
          action={closeAction}
          form="line"
          onCloseKeepWorktree={() =>
            feedback?.closeWithWorktree(closeAction.id)
          }
          onRetry={() => feedback?.retry(closeAction.id)}
        />
      )}
    </li>
  )
}
