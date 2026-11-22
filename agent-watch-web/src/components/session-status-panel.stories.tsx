import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { ReviewDocumentPanel } from '#components/review-document-panel'
import { BackgroundTaskRowView } from '#components/running-task-row-view'
import { RunningTasksPanelView } from '#components/running-tasks-panel-view'
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

const defaultStatusViewport = (Story: ComponentType) => (
  <div style={{ width: '420px', height: '600px', display: 'flex' }}>
    <Story />
  </div>
)

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
} satisfies Meta<typeof SessionStatusPanel>

export default meta
type Story = StoryObj<typeof meta>

export const CritIsTheActiveTab: Story = {
  name: 'The panel shows crit as the active tab',
  decorators: [defaultStatusViewport],
}

export const ReviewDocumentIsActive: Story = {
  name: 'The panel shows an active Review tab with typed frontmatter',
  decorators: [defaultStatusViewport],
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
  decorators: [defaultStatusViewport],
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
  decorators: [defaultStatusViewport],
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
  decorators: [defaultStatusViewport],
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
  decorators: [defaultStatusViewport],
  args: {
    tabs: [{ id: 'crit', label: 'crit', count: 0, content: null }],
    activeTabId: null,
  },
}

const statusTasks = (openCount: number, completedCount: number) => [
  ...Array.from({ length: openCount }, (_, index) => ({
    id: `status-open-task-${String(index + 1)}`,
    number: 2048 + index,
    title:
      [
        '通知の信頼性を上げる',
        '再送のテストを足す',
        '送信キューを分ける',
        '配信チェックを入れる',
      ][index] ?? '通知の再送処理を見直す',
    priority: index === 0 ? 1 : 2,
  })),
  ...Array.from({ length: completedCount }, (_, index) => ({
    id: `status-closed-task-${String(index + 1)}`,
    number: [2047, 2046, 2043, 2041][index % 4] ?? 2047,
    title:
      [
        '再送の上限を設定に出す',
        '送信キューを分ける',
        '失敗時の記録を残す',
        '再送のテストを足す',
      ][index % 4] ?? '再送の上限を設定に出す',
    priority: index % 3 === 0 ? 1 : null,
    isClosed: true,
    updatedAt: new Date(
      Date.now() - ([1, 2, 24, 48][index % 4] ?? 1) * 60 * 60 * 1_000,
    ).toISOString(),
  })),
]

type StatusStoryOptions = {
  size?: 'desktop' | 'phone'
  treeKeyboardFocus?: boolean
  openTaskCount?: number
  reviewView?: 'edit' | 'preview'
}

const runningStatusContent = (
  <RunningTasksPanelView
    subagents={
      [
        {
          id: 'fixture-agent-task',
          description: '再送処理の呼び出し元を探す',
          elapsedSeconds: 42,
          kind: 'subagent',
          agentType: 'Explore',
        },
      ] satisfies BackgroundTask[]
    }
    backgroundTaskRows={[
      <BackgroundTaskRowView
        key="fixture-shell-task"
        task={{
          id: 'fixture-shell-task',
          description: 'cargo test -p service',
          elapsedSeconds: 65,
          kind: 'background_task',
        }}
        isOpen
        output={[
          'running 24 tests',
          'test retry::stops_after_limit ... ok',
          'test retry::backs_off ... ok',
        ].join('\n')}
        error={null}
        onToggle={() => undefined}
      />,
    ]}
  />
)

const statusTabs = ({
  size = 'desktop',
  treeKeyboardFocus = false,
  openTaskCount = 1,
  reviewView = 'edit',
}: StatusStoryOptions = {}) => [
  {
    id: 'running',
    label: 'Running',
    count: 2,
    content: runningStatusContent,
  },
  {
    id: 'tasks',
    label: 'Tasks',
    count: openTaskCount + 12,
    showCount: true,
    content: (
      <SessionTasksPanel
        repo="example/service"
        primaryTaskId="status-open-task-1"
        onMakePrimary={() => Promise.resolve(true)}
        tasks={statusTasks(openTaskCount, 12) satisfies SessionTask[]}
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
        size={size}
        viewState={
          treeKeyboardFocus
            ? {
                expandedSessionIds: ['root-session', 'selected-session'],
                focusedSessionId: 'selected-session',
                treeHasFocus: true,
              }
            : null
        }
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
              title: '通知まわりの改修',
              preview: '通知の信頼性を上げる',
              elapsedSeconds: 720,
              task: {
                id: 'status-tree-task-1',
                number: 2048,
                title: '通知の信頼性を上げる',
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
              title: '通知の再送処理を見直す',
              preview: '再送処理を retry() にまとめました',
              elapsedSeconds: 8,
              task: {
                id: 'status-tree-task-2',
                number: 2048,
                title: '通知の信頼性を上げる',
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
              title: '再送の記録を残す',
              preview: '失敗時の記録を残す',
              elapsedSeconds: 60,
              task: {
                id: 'status-tree-task-3',
                number: 2048,
                title: '通知の信頼性を上げる',
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
              title: '配信チェックの PR を確認する',
              preview: '配信チェックを入れる',
              elapsedSeconds: 120,
              task: {
                id: 'status-tree-task-4',
                number: 2050,
                title: '配信チェックを入れる',
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
              title: '送信キューを分ける',
              preview: '送信キューを分けました',
              elapsedSeconds: 7200,
              task: {
                id: 'status-tree-task-5',
                number: 2048,
                title: '通知の信頼性を上げる',
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
        frontmatter={{ title: '再送処理の設計', status: 'draft' }}
        body={
          '## 方針\n\n失敗したら待ち時間を倍にして\n最大 3 回まで送り直す。'
        }
        unsavedChanges
        view={reviewView}
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
              title: '待ち時間を指数的に伸ばす',
              why: '相手のサーバーが落ちているときに負荷をかけないため。',
              considered: ['一定間隔で再試行する'],
              easyToChange: '間隔の計算は 1 か所にまとまっている。',
              review: null,
              createdAt: new Date(
                phoneStatusTimeReference - 34 * 60 * 1_000,
              ).toISOString(),
            },
            {
              type: 'decision',
              seq: 2,
              title: '再送の記録を既存のテーブルに足す',
              why: '最初のリリースでは短い履歴だけが必要です。',
              considered: ['配信経路ごとに記録を分ける'],
              easyToChange: '保存先のテーブルは adapter で切り替えられます。',
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
            title: '通知の再送処理を見直す',
            preview: '通知の再送処理について相談しています。',
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
    id: 'web-ui-crit',
    label: 'crit',
    count: 1,
    content: (
      <WebUiPane
        name="crit"
        url="http://127.0.0.1:3000/review/example"
        targetUrlOverride="http://127.0.0.1:3000/review/example"
        previewMessage="The web UI shows here."
        size={size}
        onClose={() => undefined}
      />
    ),
  },
]

const phoneStatusTabs = statusTabs({ size: 'phone', openTaskCount: 2 })
const phoneStatusManyTabs = statusTabs({ size: 'phone', openTaskCount: 4 })

const phoneStatusViewport = (Story: ComponentType) => (
  <div
    className="story-mobile-layout"
    style={{ width: '390px', height: '844px', display: 'flex' }}
  >
    <Story />
  </div>
)

const desktopStatusViewport = (Story: ComponentType) => (
  <div style={{ width: '640px', height: '900px', display: 'flex' }}>
    <Story />
  </div>
)

const phoneStatusStoryParameters = {
  overflowCheck: {
    ignoreSelectors: ['.session-status-panel-tablist'],
  },
}

const desktopStatusStory = (
  activeTabId: string,
  options: Omit<StatusStoryOptions, 'size'> = {},
) =>
  ({
    args: {
      tabs: statusTabs({ ...options, size: 'desktop' }),
      activeTabId,
    },
    decorators: [desktopStatusViewport],
  }) satisfies Story

export const StatusRunning: Story = {
  ...desktopStatusStory('running'),
  name: 'The desktop status panel shows running tasks.',
}

export const StatusTasks: Story = {
  ...desktopStatusStory('tasks', { openTaskCount: 2 }),
  name: 'The desktop status panel shows linked tasks.',
}

export const StatusTaskLinks: Story = {
  ...desktopStatusStory('tasks', { openTaskCount: 3 }),
  name: 'The desktop status panel shows multiple linked tasks.',
}

export const StatusTree: Story = {
  ...desktopStatusStory('tree'),
  name: 'The desktop status panel shows the session tree.',
}

export const StatusTreeKeyboardFocus: Story = {
  name: 'The desktop status tree shows its keyboard focus.',
  args: {
    tabs: statusTabs({ treeKeyboardFocus: true }),
    activeTabId: 'tree',
  },
  decorators: [desktopStatusViewport],
}

export const StatusReview: Story = {
  ...desktopStatusStory('review-document'),
  name: 'The desktop status panel shows the Review tab.',
}

export const StatusReviewPreview: Story = {
  ...desktopStatusStory('review-document', { reviewView: 'preview' }),
  name: 'The desktop status panel shows the Review preview.',
}

export const StatusDecisions: Story = {
  ...desktopStatusStory('decisions'),
  name: 'The desktop status panel shows pending decisions.',
}

export const StatusWebUi: Story = {
  ...desktopStatusStory('web-ui-crit'),
  name: 'The desktop status panel shows a Web UI tab.',
}

export const PhoneStatusRunning: Story = {
  name: 'The phone status page shows running tasks',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusTabs,
    activeTabId: 'running',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTasks: Story = {
  name: 'The phone status page shows linked tasks',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusTabs,
    activeTabId: 'tasks',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTasksMany: Story = {
  name: 'The phone status page shows several linked tasks',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusManyTabs,
    activeTabId: 'tasks',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusTree: Story = {
  name: 'The phone status page shows the session tree',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusTabs,
    activeTabId: 'tree',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusReview: Story = {
  name: 'The phone status page shows the Review tab',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusTabs,
    activeTabId: 'review-document',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusReviewPreview: Story = {
  name: 'The phone status page shows the Review preview',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: statusTabs({ size: 'phone', reviewView: 'preview' }),
    activeTabId: 'review-document',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusDecisions: Story = {
  name: 'The phone status page shows pending decisions',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusTabs,
    activeTabId: 'decisions',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}

export const PhoneStatusWebUi: Story = {
  name: 'The phone status page shows a Web UI tab',
  args: {
    size: 'phone',
    sessionTitle: '通知の信頼性を上げる',
    tabs: phoneStatusTabs,
    activeTabId: 'web-ui-crit',
  },
  decorators: [phoneStatusViewport],
  parameters: phoneStatusStoryParameters,
}
