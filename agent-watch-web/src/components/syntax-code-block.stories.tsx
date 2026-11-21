import type { Meta, StoryObj } from '@storybook/react-vite'

import { SyntaxCodeBlock } from '#components/syntax-code-block'

const meta = {
  component: SyntaxCodeBlock,
  args: {
    language: 'bash',
    copyable: true,
    code: `set -euo pipefail
printf '%s\\n' 'Preparing preview'
git add ./sample-config.yml && git commit -m "$(cat <<'EOF'
chore: refresh sample config

intent(sample): keep local settings in sync
EOF
)"`,
  },
  decorators: [
    (Story) => (
      <div
        style={{ maxWidth: '640px', padding: '24px', background: '#0a0a0a' }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SyntaxCodeBlock>

export default meta
type Story = StoryObj<typeof meta>

export const BashHighlightsACommand: Story = {
  name: 'A Bash block keeps heredocs and line breaks visible',
}

export const WithoutCopyToolbar: Story = {
  name: 'The block renders without a copy toolbar',
  args: {
    copyable: false,
  },
}

export const UnspecifiedLanguageUsesText: Story = {
  name: 'A block without a language label is identified as text',
  args: {
    code: 'A plain text block can still be copied.',
    language: 'text',
  },
}

export const LongLinesScrollBelowTheToolbar: Story = {
  name: 'A long line scrolls below the fixed toolbar',
  parameters: {
    overflowCheck: { ignoreSelectors: ['.syntax-code-block'] },
  },
  args: {
    code: `const preview = buildPreview({ retries: 3, timeoutMs: 12000, endpoint: 'https://example.invalid/api/preview', includeMetadata: true, includeDiagnostics: true, format: 'compact' })`,
    language: 'typescript',
  },
}
