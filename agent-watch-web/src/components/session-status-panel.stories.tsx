import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { ReviewDocumentPanel } from '#components/review-document-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import { SessionStatusPanel } from '#components/session-status-panel'
import { SessionTasksPanel } from '#components/session-tasks-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import type {
  BackgroundTask,
  Session,
  SessionTask,
  SessionTreeNode,
} from '#types'

const meta = {
  component: SessionStatusPanel,
  args: {
    tabs: [
      {
        id: 'crit',
        label: 'crit',
        count: 1,
        content: <div className="session-status-preview">Crit review</div>,
      },
    ],
    activeTabId: 'crit',
    onTabChange: () => undefined,
    onClose: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '420px', height: '600px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionStatusPanel>

export default meta
type Story = StoryObj<typeof meta>

export const CritIsTheActiveTab: Story = {
  name: 'The panel shows crit as the active tab',
}

export const ReviewDocumentIsActive: Story = {
  name: 'The panel shows an active Review tab with typed frontmatter',
  args: {
    tabs: [
      {
        id: 'review-document',
        label: 'Review',
        count: 0,
        visible: true,
        showCount: false,
        content: (
          <ReviewDocumentPanel
            frontmatter={{
              title: 'Review the retry policy',
              enabled: true,
              steps: { publish: false },
            }}
            body={'## Summary\n\nReview the retry behavior.'}
          />
        ),
      },
    ],
    activeTabId: 'review-document',
  },
}

export const OneRunningItemIsAvailable: Story = {
  name: 'The panel shows a count for one running item',
  args: {
    tabs: [
      {
        id: 'running',
        label: 'Running',
        count: 1,
        content: <div className="session-status-preview">One running task</div>,
      },
    ],
    activeTabId: 'running',
  },
}

export const DecisionsShowUnreviewedCount: Story = {
  name: 'The Decisions tab shows how many decisions need review',
  args: {
    tabs: [
      {
        id: 'decisions',
        label: 'Decisions',
        count: 2,
        visible: true,
        showCount: true,
        countTone: 'warning',
        content: (
          <div className="session-status-preview">
            Decisions waiting for review
          </div>
        ),
      },
    ],
    activeTabId: 'decisions',
  },
}

export const ManyTabsKeepCloseActionVisible: Story = {
  name: 'The panel scrolls many tabs while keeping the close action visible',
  parameters: {
    overflowCheck: {
      ignoreSelectors: ['.session-status-panel-tablist'],
    },
  },
  args: {
    tabs: [
      'Running',
      'Browser preview',
      'Metrics dashboard',
      'Review queue',
      'Workspace integration with a long descriptive label',
      'Build output',
      'Deployment status',
      'Additional service view',
    ].map((label, index) => ({
      id: `view-${String(index + 1)}`,
      label,
      count: 1,
      content: <div className="session-status-preview">{label}</div>,
    })),
    activeTabId: 'view-8',
  },
}

export const NoTabsAreAvailable: Story = {
  name: 'The panel shows an empty state when no tabs have items',
  args: {
    tabs: [{ id: 'crit', label: 'crit', count: 0, content: null }],
    activeTabId: null,
  },
}

const phoneStatusTabs = [
  {
    id: 'running',
    label: 'Running',
    count: 2,
    content: (
      <RunningTasksPanel
        sessionId="fixture-session"
        tasks={
          [
            {
              id: 'fixture-agent-task',
              description: 'Inspect the fictional module structure',
              elapsedSeconds: 72,
              kind: 'subagent',
              agentType: 'FixtureExplorer',
            },
            {
              id: 'fixture-shell-task',
              description: 'cargo test -p fictional-crate',
              elapsedSeconds: 31,
              kind: 'background_task',
            },
          ] satisfies BackgroundTask[]
        }
      />
    ),
  },
  {
    id: 'tasks',
    label: 'Tasks',
    count: 2,
    showCount: true,
    content: (
      <SessionTasksPanel
        repo="example/service"
        primaryTaskId="example-open-task"
        onMakePrimary={() => Promise.resolve(true)}
        tasks={
          [
            {
              id: 'example-open-task',
              number: 2048,
              title: 'Improve request reliability',
              priority: 1,
            },
            {
              id: 'example-completed-task',
              number: 2047,
              title: 'Document request behavior',
              priority: null,
              isClosed: true,
              updatedAt: '2026-11-21T12:00:00Z',
            },
          ] satisfies SessionTask[]
        }
      />
    ),
  },
  {
    id: 'tree',
    label: 'Tree',
    count: 1,
    showCount: true,
    content: (
      <SessionTreePanel
        selectedSessionId="selected-session"
        onSelectSession={() => undefined}
        sessions={
          [
            {
              id: 'root-session',
              parentId: null,
              engine: 'claude',
              workType: 'PM',
              category: 'in_progress',
              isActive: true,
              kind: null,
              title: 'Root session',
              preview: 'Coordinating the task',
              elapsedSeconds: 300,
              task: null,
              repo: 'example/service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'selected-session',
              parentId: 'root-session',
              engine: 'codex',
              workType: 'development',
              category: 'in_progress',
              isActive: true,
              kind: null,
              title: 'Selected session',
              preview: 'Implementing the interface',
              elapsedSeconds: 45,
              task: null,
              repo: 'example/service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'pending-session',
              parentId: 'selected-session',
              engine: 'claude',
              workType: 'research',
              category: 'needs_action',
              isActive: true,
              kind: 'Question',
              title: 'Pending child',
              preview: 'Waiting for an answer',
              elapsedSeconds: 8,
              task: null,
              repo: 'example/other-service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
          ] satisfies SessionTreeNode[]
        }
      />
    ),
  },
  {
    id: 'review-document',
    label: 'Review',
    count: 0,
    visible: true,
    showCount: false,
    content: (
      <ReviewDocumentPanel
        frontmatter={{ title: 'Review the retry policy', enabled: true }}
        body={'## Summary\n\nReview the retry behavior.'}
      />
    ),
  },
  {
    id: 'decisions',
    label: 'Decisions',
    count: 1,
    showCount: true,
    countTone: 'warning' as const,
    content: (
      <DecisionsPanel
        decisions={
          [
            {
              type: 'decision',
              seq: 1,
              title: 'Use a fixed retry limit',
              why: 'The current service has no reliable timing data.',
              considered: ['Retry using the latest duration'],
              easyToChange: 'One configuration value in the CI workflow.',
              review: null,
              createdAt: '2026-11-21T12:00:00Z',
            },
          ] satisfies Decision[]
        }
        session={
          {
            id: 'fixture-session',
            category: 'in_progress',
            isPaused: false,
            kind: null,
            title: 'Set a retry policy',
            preview: null,
            elapsedSeconds: 120,
            unread: false,
            task: null,
            repo: 'example/service',
            project: 'service reliability',
            parent: null,
            ancestorSessionIds: [],
            pullRequest: null,
          } satisfies Session
        }
        selectedSeq={1}
        onSelect={() => undefined}
        onOk={() => Promise.resolve(true)}
        onChange={() => Promise.resolve(true)}
      />
    ),
  },
  {
    id: 'preview',
    label: 'crit',
    count: 1,
    content: (
      <div className="session-status-preview">
        <strong>Web UI preview</strong>
        <p>A registered session interface fills this panel.</p>
      </div>
    ),
  },
]

const phoneStatusViewport = (Story: ComponentType) => (
  <div
    className="story-mobile-layout"
    style={{ width: '390px', height: '844px', display: 'flex' }}
  >
    <Story />
  </div>
)

const phoneStatusStoryParameters = {
  overflowCheck: {
    ignoreSelectors: ['.session-status-panel-tablist'],
  },
}

export const PhoneStatusRunning: Story = {
  name: 'The phone status page shows running tasks',
  args: {
    mobilePresentation: true,
    tabs: phoneStatusTabs,
    activeTabId: 'running',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTasks: Story = {
  name: 'The phone status page shows linked tasks',
  args: {
    mobilePresentation: true,
    tabs: phoneStatusTabs,
    activeTabId: 'tasks',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTree: Story = {
  name: 'The phone status page shows the session tree',
  args: {
    mobilePresentation: true,
    tabs: phoneStatusTabs,
    activeTabId: 'tree',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusReview: Story = {
  name: 'The phone status page shows the Review tab',
  args: {
    mobilePresentation: true,
    tabs: phoneStatusTabs,
    activeTabId: 'review-document',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusDecisions: Story = {
  name: 'The phone status page shows pending decisions',
  args: {
    mobilePresentation: true,
    tabs: phoneStatusTabs,
    activeTabId: 'decisions',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusWebUi: Story = {
  name: 'The phone status page shows a Web UI tab',
  args: {
    mobilePresentation: true,
    tabs: phoneStatusTabs,
    activeTabId: 'preview',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}
