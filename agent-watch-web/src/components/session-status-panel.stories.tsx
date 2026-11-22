import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { ReviewDocumentPanel } from '#components/review-document-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import { SessionStatusPanel } from '#components/session-status-panel'
import { SessionTasksPanel } from '#components/session-tasks-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import { WebUiPane } from '#components/web-ui-pane'
import type {
  BackgroundTask,
  Session,
  SessionTask,
  SessionTreeNode,
} from '#types'

const phoneStatusTimeReference = Date.now()

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

const phoneStatusTasks = (openCount: number, completedCount: number) => [
  ...Array.from({ length: openCount }, (_, index) => ({
    id: `phone-open-task-${String(index + 1)}`,
    number: 7301 + index,
    title:
      [
        'Improve delivery reliability',
        'Add retry metrics',
        'Document queue behavior',
        'Review alert thresholds',
      ][index] ?? 'Refine the delivery workflow',
    priority: index === 0 ? 1 : 2,
  })),
  ...Array.from({ length: completedCount }, (_, index) => ({
    id: `phone-closed-task-${String(index + 1)}`,
    number: 7201 + index,
    title:
      [
        'Add request tracing',
        'Document retry limits',
        'Separate delivery queues',
        'Cover the timeout path',
      ][index % 4] ?? 'Improve service diagnostics',
    priority: index % 3 === 0 ? 1 : null,
    isClosed: true,
    updatedAt: new Date(
      Date.now() - ([1, 2, 24, 48][index % 4] ?? 1) * 60 * 60 * 1_000,
    ).toISOString(),
  })),
]

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
              description: 'Inspect retry behavior',
              elapsedSeconds: 42,
              kind: 'subagent',
              agentType: 'Research',
            },
            {
              id: 'fixture-shell-task',
              description: 'cargo test -p fictional-crate',
              elapsedSeconds: 65,
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
    count: 13,
    showCount: true,
    content: (
      <SessionTasksPanel
        repo="example/service"
        primaryTaskId="phone-open-task-1"
        onMakePrimary={() => Promise.resolve(true)}
        tasks={phoneStatusTasks(1, 12) satisfies SessionTask[]}
      />
    ),
  },
  {
    id: 'tree',
    label: 'Tree',
    count: 1,
    showCount: true,
    countTone: 'warning' as const,
    content: (
      <SessionTreePanel
        mobilePresentation
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
              title: 'Delivery workflow',
              preview: 'Coordinating the task',
              elapsedSeconds: 720,
              task: {
                id: 'phone-tree-task-1',
                number: 7301,
                title: 'Improve delivery reliability',
                priority: 1,
              },
              repo: 'example/service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'selected-session',
              parentId: 'root-session',
              engine: 'claude',
              workType: 'development',
              category: 'needs_action',
              isActive: true,
              kind: 'Question',
              title: 'Review delivery retries',
              preview: 'Check the retry behavior',
              elapsedSeconds: 8,
              task: {
                id: 'phone-tree-task-2',
                number: 7302,
                title: 'Review retry behavior',
                priority: 2,
              },
              repo: 'example/service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'record-session',
              parentId: 'selected-session',
              engine: 'codex',
              workType: 'development',
              category: 'in_progress',
              isActive: true,
              kind: null,
              title: 'Record retry attempts',
              preview: 'Add delivery metrics',
              elapsedSeconds: 60,
              task: {
                id: 'phone-tree-task-3',
                number: 7303,
                title: 'Add retry metrics',
                priority: 1,
              },
              repo: 'example/service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'checks-session',
              parentId: 'root-session',
              engine: 'codex',
              workType: 'research',
              category: 'needs_action',
              isActive: true,
              kind: 'Review',
              title: 'Review rollout checks',
              preview: 'Confirm the release checks',
              elapsedSeconds: 120,
              task: {
                id: 'phone-tree-task-4',
                number: 7304,
                title: 'Check release process',
                priority: 2,
              },
              repo: 'example/service',
              vibeMode: { mode: 'disabled' },
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'queue-session',
              parentId: 'root-session',
              engine: 'claude',
              workType: 'development',
              category: 'done',
              isActive: false,
              kind: null,
              title: 'Separate delivery queues',
              preview: 'The queue change is complete',
              elapsedSeconds: 7200,
              task: {
                id: 'phone-tree-task-5',
                number: 7305,
                title: 'Separate delivery queues',
                priority: 1,
              },
              repo: 'example/service',
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
        frontmatter={{ title: 'Review the delivery policy', status: 'draft' }}
        body={
          '## Policy\n\nUse longer delays after failures\nStop after three attempts.'
        }
        unsavedChanges
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
              why: 'A capped retry budget prevents repeated deliveries from overloading the receiving service.',
              considered: ['Retry using the latest duration'],
              easyToChange:
                'The retry interval comes from one worker configuration setting.',
              review: null,
              createdAt: new Date(
                phoneStatusTimeReference - 34 * 60 * 1_000,
              ).toISOString(),
            },
            {
              type: 'decision',
              seq: 2,
              title: 'Keep delivery history in one table',
              why: 'The first release only needs a short audit trail.',
              considered: ['Split records by delivery channel'],
              easyToChange: 'The persistence adapter owns the table mapping.',
              review: 'ok',
              createdAt: new Date(
                phoneStatusTimeReference - 60 * 60 * 1_000,
              ).toISOString(),
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
        relativeTimeReference={phoneStatusTimeReference}
        showReviewed
        selectedSeq={1}
        onSelect={() => undefined}
        onOk={() => Promise.resolve(true)}
        onChange={() => Promise.resolve(true)}
      />
    ),
  },
  {
    id: 'web-ui-preview',
    label: 'Preview',
    count: 1,
    content: (
      <WebUiPane
        name="Preview"
        url="https://example.test/dashboard"
        targetUrlOverride="https://example.test/dashboard"
        previewMessage="The web interface is shown here."
        mobileTabPresentation
        onClose={() => undefined}
      />
    ),
  },
]

const phoneStatusManyTabs = phoneStatusTabs.map((tab) =>
  tab.id === 'tasks'
    ? {
        ...tab,
        count: 16,
        content: (
          <SessionTasksPanel
            repo="example/service"
            primaryTaskId="phone-open-task-1"
            onMakePrimary={() => Promise.resolve(true)}
            tasks={phoneStatusTasks(4, 12) satisfies SessionTask[]}
          />
        ),
      }
    : tab,
)

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
    sessionTitle: 'Improve delivery reliability',
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
    sessionTitle: 'Improve delivery reliability',
    tabs: phoneStatusTabs,
    activeTabId: 'tasks',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTasksMany: Story = {
  name: 'The phone status page shows several linked tasks',
  args: {
    mobilePresentation: true,
    sessionTitle: 'Improve delivery reliability',
    tabs: phoneStatusManyTabs,
    activeTabId: 'tasks',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTree: Story = {
  name: 'The phone status page shows the session tree',
  args: {
    mobilePresentation: true,
    sessionTitle: 'Improve delivery reliability',
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
    sessionTitle: 'Improve delivery reliability',
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
    sessionTitle: 'Improve delivery reliability',
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
    sessionTitle: 'Improve delivery reliability',
    tabs: phoneStatusTabs,
    activeTabId: 'web-ui-preview',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}
