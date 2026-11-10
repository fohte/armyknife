import type { Meta, StoryObj } from '@storybook/react-vite'

import { buildBoardProjects } from '#board-model'
import { BoardView } from '#components/board-view'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'session-question',
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    title: 'Review the updated API contract',
    preview: 'Can the response include a project name?',
    elapsedSeconds: 8,
    unread: true,
    task: {
      id: 'task-api-contract',
      number: 2048,
      title: 'Update the service contract',
      priority: 1,
    },
    repo: 'example/service',
    project: 'service refresh',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
  {
    id: 'session-working',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Add request metrics',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'service refresh',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
  {
    id: 'session-done',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Document request limits',
    preview: null,
    elapsedSeconds: 3600,
    unread: false,
    task: null,
    repo: 'example/service',
    project: 'service refresh',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: { number: 2049, state: 'Merged' },
  },
  {
    id: 'session-quiet',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Record deployment notes',
    preview: null,
    elapsedSeconds: 7200,
    unread: false,
    task: null,
    repo: 'example/docs',
    project: 'documentation update',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
]

const projects = buildBoardProjects(sessions)

const meta = {
  component: BoardView,
  args: {
    projects,
    loaded: true,
    error: null,
    hideQuietProjects: true,
    onHideQuietProjectsChange: () => undefined,
    onSelectSession: () => undefined,
  },
  decorators: [
    (Story) => (
      <div className="story-board-container">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof BoardView>

export default meta
type Story = StoryObj<typeof meta>

export const BoardShowsActiveProjectsFirst: Story = {
  name: 'The board puts projects that need attention first',
}

export const BoardShowsPausedSessions: Story = {
  name: 'The Done group identifies paused sessions',
  args: {
    projects: buildBoardProjects([
      ...sessions,
      {
        id: 'session-paused',
        category: 'done',
        isPaused: true,
        kind: null,
        title: 'Prepare the next deployment window',
        preview: null,
        elapsedSeconds: 240,
        unread: false,
        task: null,
        repo: 'example/service',
        project: 'service refresh',
        ancestorSessionIds: [],
        parent: null,
        pullRequest: null,
      },
    ]),
  },
}

export const BoardShowsQuietProjects: Story = {
  name: 'The board includes quiet projects when the filter is off',
  args: { hideQuietProjects: false },
}

export const BoardHasNoProjects: Story = {
  name: 'The board shows an empty state when no projects are available',
  args: { projects: [] },
}

export const BoardIsLoading: Story = {
  name: 'The board shows a loading state before projects arrive',
  args: { projects: [], loaded: false },
}

export const BoardShowsRefreshError: Story = {
  name: 'The board keeps showing projects when metadata refresh fails',
  args: { error: 'Project metadata could not be refreshed.' },
}
