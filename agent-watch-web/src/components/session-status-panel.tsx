import '#components/session-status-panel.css'

import { Button } from '@fohte/ui/button'
import type { ReactNode } from 'react'

export type SessionStatusPanelTab = {
  id: string
  label: string
  count: number
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
  const visibleTabs = tabs.filter((tab) => tab.count > 0)
  const activeTab = visibleTabs.find((tab) => tab.id === activeTabId)

  return (
    <aside
      id="session-status-panel"
      className="session-status-panel"
      aria-label="Session status panel"
    >
      <div
        className="session-status-panel-tabs"
        role="tablist"
        aria-label="Session status views"
      >
        {visibleTabs.map((tab) => (
          <button
            key={tab.id}
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
            {tab.count > 1 ? (
              <span className="session-status-panel-count">{tab.count}</span>
            ) : null}
          </button>
        ))}
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
