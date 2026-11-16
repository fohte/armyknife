import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionDirectoryOptions } from '#components/session-directory-options'
import { exampleSessionDirectories } from '#new-session-story-fixtures.test'

const meta = {
  component: SessionDirectoryOptions,
  args: {
    directories: exampleSessionDirectories,
    isLoading: false,
    query: '',
    onQueryChange: () => undefined,
    onSelect: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '380px', border: '1px solid var(--aw-border)' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionDirectoryOptions>

export default meta
type Story = StoryObj<typeof meta>

export const RecentProjectsAndPathsAreGrouped: Story = {
  name: 'Recent directories, projects, and paths use separate groups',
}

export const DirectoriesAreLoading: Story = {
  name: 'The directory results show a loading state',
  args: { directories: [], isLoading: true },
}

export const SearchHasNoMatches: Story = {
  name: 'The directory results show when a search has no matches',
  args: { directories: [], query: 'unknown' },
}

export const DirectorySearchFailed: Story = {
  name: 'The directory results explain when the directory request fails',
  args: {
    directories: [],
    error: 'Could not load directory suggestions.',
  },
}
