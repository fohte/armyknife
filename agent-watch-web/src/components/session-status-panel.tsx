import '#components/session-status-panel.css'

import { Button } from '@fohte/ui/button'
import { ChevronLeft } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { useFocusTrap } from '#components/use-focus-trap'

export type SessionStatusPanelTab = {
  id: string
  label: string
  count: number
  visible?: boolean
  showCount?: boolean
  countTone?: 'running' | 'warning'
  content: ReactNode
}

type SessionStatusPanelProps = {
  tabs: readonly SessionStatusPanelTab[]
  activeTabId: string | null
  onTabChange: (id: string) => void
  onClose: () => void
  sessionTitle?: string
  mobilePresentation?: boolean
}

export function SessionStatusPanel({
  tabs,
  activeTabId,
  onTabChange,
  onClose,
  sessionTitle,
  mobilePresentation = false,
}: SessionStatusPanelProps) {
  const visibleTabs = tabs.filter((tab) => tab.visible ?? tab.count > 0)
  const visibleTabSignature = visibleTabs
    .map((tab) => `${tab.id}:${tab.label}:${String(tab.count)}`)
    .join('|')
  const activeTab = visibleTabs.find((tab) => tab.id === activeTabId)
  const panelRef = useRef<HTMLElement>(null)
  const tabListRef = useRef<HTMLDivElement>(null)
  const activeTabRef = useRef<HTMLButtonElement>(null)
  const backButtonRef = useRef<HTMLButtonElement>(null)
  const [hasHiddenTabs, setHasHiddenTabs] = useState({
    left: false,
    right: false,
  })
  useFocusTrap({
    containerRef: panelRef,
    initialFocusRef: backButtonRef,
    isActive: mobilePresentation,
  })

  const updateHiddenTabs = useCallback(() => {
    const tabList = tabListRef.current
    if (tabList === null) return
    const startPadding = Number.parseFloat(
      getComputedStyle(tabList).paddingLeft,
    )
    setHasHiddenTabs({
      left: tabList.scrollLeft > startPadding,
      right: tabList.scrollLeft + tabList.clientWidth < tabList.scrollWidth - 1,
    })
  }, [])

  useEffect(() => {
    const tabList = tabListRef.current
    const activeTabElement = activeTabRef.current
    if (tabList === null || activeTabElement === null) return

    const tabListBounds = tabList.getBoundingClientRect()
    const activeTabBounds = activeTabElement.getBoundingClientRect()
    const visibleLeft = tabListBounds.left + tabList.clientLeft
    const visibleRight = visibleLeft + tabList.clientWidth

    if (activeTabBounds.left < visibleLeft) {
      tabList.scrollBy({ left: activeTabBounds.left - visibleLeft })
    } else if (activeTabBounds.right > visibleRight - 40) {
      tabList.scrollBy({ left: activeTabBounds.right - visibleRight + 40 })
    }
    updateHiddenTabs()
  }, [activeTabId, updateHiddenTabs])

  useEffect(() => {
    const tabList = tabListRef.current
    if (tabList === null) return
    updateHiddenTabs()
    tabList.addEventListener('scroll', updateHiddenTabs, { passive: true })
    const resizeObserver = new ResizeObserver(updateHiddenTabs)
    resizeObserver.observe(tabList)
    tabList
      .querySelectorAll<HTMLButtonElement>('[role="tab"]')
      .forEach((tab) => {
        resizeObserver.observe(tab)
      })
    return () => {
      tabList.removeEventListener('scroll', updateHiddenTabs)
      resizeObserver.disconnect()
    }
  }, [updateHiddenTabs, visibleTabSignature])

  return (
    <aside
      ref={panelRef}
      id="session-status-panel"
      className={`session-status-panel${mobilePresentation ? ' session-status-panel--mobile' : ''}`}
      aria-label="Session status panel"
      aria-labelledby={
        mobilePresentation ? 'session-status-panel-title' : undefined
      }
      role={mobilePresentation ? 'dialog' : undefined}
      aria-modal={mobilePresentation || undefined}
      tabIndex={mobilePresentation ? -1 : undefined}
    >
      {mobilePresentation ? (
        <header className="session-status-panel-mobile-header">
          <Button
            ref={backButtonRef}
            className="session-status-panel-back"
            type="button"
            variant="plain"
            aria-label="Back to the conversation"
            onClick={() => {
              onClose()
            }}
          >
            <ChevronLeft
              aria-hidden="true"
              size={30}
              strokeWidth={2.5}
              style={{ width: '30px', height: '30px' }}
            />
          </Button>
          <div className="session-status-panel-mobile-heading">
            <h1 id="session-status-panel-title">Session status</h1>
            {sessionTitle === undefined ? null : (
              <span title={sessionTitle}>{sessionTitle}</span>
            )}
          </div>
        </header>
      ) : null}
      <div className="session-status-panel-tabs">
        <div
          className={`session-status-panel-tablist-wrap${hasHiddenTabs.left ? ' has-hidden-tabs-left' : ''}${hasHiddenTabs.right ? ' has-hidden-tabs-right' : ''}`}
        >
          <div
            className="session-status-panel-tablist"
            ref={tabListRef}
            role="tablist"
            aria-label="Session status views"
          >
            {visibleTabs.map((tab) => (
              <button
                key={tab.id}
                ref={tab.id === activeTabId ? activeTabRef : undefined}
                id={`session-status-tab-${tab.id}`}
                type="button"
                role="tab"
                aria-controls={`session-status-panel-${tab.id}`}
                aria-selected={activeTabId === tab.id}
                className="session-status-panel-tab"
                onClick={() => {
                  onTabChange(tab.id)
                }}
              >
                <span>{tab.label}</span>
                {(tab.showCount ?? (tab.id === 'running' || tab.count > 1)) ? (
                  <span
                    className={`session-status-panel-count${tab.countTone === 'running' || (tab.countTone === undefined && tab.id === 'running') ? ' session-status-panel-count--running' : ''}${tab.countTone === 'warning' ? ' session-status-panel-count--warning' : ''}`}
                  >
                    {tab.count}
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        </div>
        {mobilePresentation ? null : (
          <Button
            className="session-status-panel-close"
            type="button"
            variant="plain"
            aria-label="Close session status panel"
            onClick={() => {
              onClose()
            }}
          >
            ×
          </Button>
        )}
      </div>
      {activeTab !== undefined ? (
        <div
          id={`session-status-panel-${activeTab.id}`}
          className="session-status-panel-content"
          role="tabpanel"
          aria-labelledby={`session-status-tab-${activeTab.id}`}
        >
          {activeTab.content}
        </div>
      ) : (
        <div className="session-status-panel-empty" role="status">
          No session status items.
        </div>
      )}
    </aside>
  )
}
