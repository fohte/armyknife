import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxFilterBar } from '#components/inbox-filter-bar'

const meta = {
  component: InboxFilterBar,
  args: {
    filters: [],
    matchingNeedsActionCount: 3,
    totalNeedsActionCount: 3,
    onFilterQueryChange: () => undefined,
    onOpenFilterDialog: () => undefined,
    onSaveViewOpenChange: () => undefined,
    onSaveViewNameChange: () => undefined,
    onSaveView: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '720px', background: '#0a0a0a' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof InboxFilterBar>

export default meta
type Story = StoryObj<typeof meta>

export const NoFiltersAreActive: Story = {
  name: 'The filter bar shows the shortcut before filters are set',
}

export const FiltersCanBeSaved: Story = {
  name: 'The filter bar shows matching counts and a save action',
  args: {
    filters: [
      { key: 'project', values: ['sample-project'] },
      { key: 'kind', values: ['question', 'review'] },
    ],
    matchingNeedsActionCount: 2,
    totalNeedsActionCount: 5,
  },
}

export const AViewNameIsBeingEntered: Story = {
  name: 'The filter bar shows the field for a saved view name',
  args: {
    filters: [
      { key: 'project', values: ['sample-project'] },
      { key: 'kind', values: ['question', 'review'] },
    ],
    matchingNeedsActionCount: 2,
    totalNeedsActionCount: 5,
    saveViewOpen: true,
    saveViewName: 'Sample questions',
  },
}
