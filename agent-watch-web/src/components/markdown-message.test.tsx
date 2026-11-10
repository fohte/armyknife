import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { MarkdownMessage } from '#components/markdown-message'

function renderMessage(markdown: string, className?: string) {
  const props = className === undefined ? {} : { className }
  return renderToStaticMarkup(
    <MarkdownMessage {...props}>{markdown}</MarkdownMessage>,
  )
}

it('renders common markdown and GFM as semantic elements', () => {
  const html = renderMessage(
    `## Summary

Use \`retry()\` with the [guide](https://example.com/guide).
First line
Second line

- Retry transient failures
- Stop after three attempts

| Attempt | Delay |
| --- | --- |
| 1 | 100 ms |

~~~rust
let attempts = 3;
~~~`,
    'chat-user-message',
  )

  const expected = [
    '<div class="markdown-message chat-user-message"><h2>Summary</h2>',
    '<p>Use <code>retry()</code> with the <a href="https://example.com/guide">guide</a>.',
    'First line',
    'Second line</p>',
    '<ul>',
    '<li>Retry transient failures</li>',
    '<li>Stop after three attempts</li>',
    '</ul>',
    '<table><thead><tr><th>Attempt</th><th>Delay</th></tr></thead><tbody><tr><td>1</td><td>100 ms</td></tr></tbody></table>',
    '<pre><code class="language-rust">let attempts = 3;',
    '</code></pre></div>',
  ].join(String.fromCharCode(10))

  expect(html).toBe(expected)
})

it('keeps raw HTML as text and blocks unsafe link protocols', () => {
  const html = renderMessage(
    '<img src=x onerror=alert(1)> [unsafe](javascript:alert(1))',
  )

  expect(html).toBe(
    '<div class="markdown-message"><p>&lt;img src=x onerror=alert(1)&gt; <a href="">unsafe</a></p></div>',
  )
})
