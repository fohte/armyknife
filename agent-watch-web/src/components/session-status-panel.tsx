import '#components/session-status-panel.css'

import { Button } from '@fohte/ui/button'
import type { ReactNode } from 'react'
import { useEffect, useRef } from 'react'

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
}

export function SessionStatusPanel({
  tabs,
  activeTabId,
  onTabChange,
  onClose,
}: SessionStatusPanelProps) {
  const visibleTabs = tabs.filter((tab) => tab.visible ?? tab.count > 0)
  const activeTab = visibleTabs.find((tab) => tab.id === activeTabId)
  const tabListRef = useRef<HTMLDivElement>(null)
  const activeTabRef = useRef<HTMLButtonElement>(null)

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
    } else if (activeTabBounds.right > visibleRight) {
      tabList.scrollBy({ left: activeTabBounds.right - visibleRight })
    }
  }, [activeTabId])

  return (
    <aside
      id="session-status-panel"
      className="session-status-panel"
      aria-label="Session status panel"
    >
      <div className="session-status-panel-tabs">
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
