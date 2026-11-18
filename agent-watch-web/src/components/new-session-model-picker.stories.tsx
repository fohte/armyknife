import type { Meta, StoryObj } from '@storybook/react-vite'

import { NewSessionModelPicker } from '#components/new-session-model-picker'
import { exampleModelOptions } from '#new-session-story-fixtures.test'

const meta = {
  component: NewSessionModelPicker,
  args: {
    engine: 'claude',
    options: exampleModelOptions,
    activeModelOption: exampleModelOptions.models[0],
    selectedModel: null,
    selectedEffort: null,
    isOpen: false,
    isLoading: false,
    error: null,
    onOpenChange: () => undefined,
    onModelChange: () => undefined,
    onEffortChange: () => undefined,
  },
} satisfies Meta<typeof NewSessionModelPicker>

export default meta
type Story = StoryObj<typeof meta>

export const DefaultControl: Story = {
  name: 'The chip shows the profile default model and effort',
  args: { variant: 'control' },
}

export const PickerIsOpen: Story = {
  name: 'The selected model exposes its available effort levels',
  args: {
    variant: 'control',
    isOpen: true,
    selectedModel: 'fixture-model-orbit',
    selectedEffort: 'high',
  },
}

export const PalettePicker: Story = {
  name: 'The command palette shows the model and effort selection view',
  args: { variant: 'palette', isOpen: true },
  decorators: [
    (Story) => (
      <div style={{ height: '420px', width: '680px' }}>
        <Story />
      </div>
    ),
  ],
}
