import '#components/message-composer.css'
import '#components/new-session-starting.css'

import { useLayoutEffect, useRef } from 'react'

import { AsyncActionItem } from '#components/async-action-item'
import { Composer } from '#components/composer'
import { UserWithContent } from '#components/user-with-content'
import type { PendingLaunchView } from '#use-session-launch'

type NewSessionStartingProps = {
  pendingLaunch: PendingLaunchView
  isMobile: boolean
  now?: Date
}

export function NewSessionStarting({
  pendingLaunch,
  isMobile,
  now,
}: NewSessionStartingProps) {
  const { request, startedAt } = pendingLaunch
  const hasMessage = (request.initialMessage?.trim().length ?? 0) > 0
  const timelineRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    timelineRef.current?.focus()
  }, [])

  return (
    <div className="new-session-starting">
      <div
        className="new-session-starting-timeline"
        ref={timelineRef}
        tabIndex={-1}
      >
        {hasMessage ? (
          <UserWithContent
            message={{
              type: 'user',
              text: request.initialMessage ?? '',
              timestamp: new Date(startedAt).toISOString(),
            }}
            {...(now === undefined ? {} : { now })}
          />
        ) : null}
        <AsyncActionItem
          action={{
            id: 0,
            kind: 'launch',
            sessionId: 'pending-launch',
            sessionTitle: request.directory,
            ...(request.initialMessage === undefined
              ? {}
              : { initialMessage: request.initialMessage }),
            engine: request.engine,
            repo: request.directory,
            status: 'launching',
            message:
              'You can leave this page. We’ll let you know when it starts.',
            createdAt: startedAt,
          }}
          form="line"
        />
      </div>
      <div
        className="conversation-composer"
        data-mobile={isMobile ? 'true' : undefined}
      >
        <Composer
          onAddAttachments={() => undefined}
          addDisabled
          sendState="dim"
          keyHint={
            isMobile ? undefined : '↵ Send · ⇧↵ / ⌥↵ New line · Esc Interrupt'
          }
          keyHint2={isMobile ? undefined : '⌥↑ / ⌥↓ My messages'}
          isMobile={isMobile}
        >
          <textarea
            aria-label="Message"
            placeholder="Reply once the session starts"
            disabled
            rows={isMobile ? 1 : 2}
          />
        </Composer>
      </div>
    </div>
  )
}
