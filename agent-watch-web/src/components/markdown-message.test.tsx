import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { MarkdownMessage } from '#components/markdown-message'

function renderMessage(
  markdown: string,
  className?: string,
  imageSources?: Readonly<Record<string, string | null>>,
) {
  const props = {
    ...(className === undefined ? {} : { className }),
    ...(imageSources === undefined ? {} : { imageSources }),
  }
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
const maxPreviewItems = 7;
~~~`,
    'chat-user-message',
  )

  const expected = [
    '<div class="markdown-message chat-user-message"><h2>Summary</h2>',
    '<p>Use <code>retry()</code> with the <a href="https://example.com/guide" target="_blank" rel="noopener noreferrer">guide</a>.',
    'First line',
    'Second line</p>',
    '<ul>',
    '<li>Retry transient failures</li>',
    '<li>Stop after three attempts</li>',
    '</ul>',
    '<table><thead><tr><th>Attempt</th><th>Delay</th></tr></thead><tbody><tr><td>1</td><td>100 ms</td></tr></tbody></table>',
    '<div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">rust</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="rust"><code class="language-rust">const maxPreviewItems = 7;',
    '</code></pre></div></div>',
  ].join(String.fromCharCode(10))

  expect(html).toBe(expected)
})

it('renders markdown images with the shared conversation image treatment', () => {
  expect(
    renderMessage('![Sample screenshot](/api/images/fixture-image.png)'),
  ).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge Sample screenshot"><img class="chat-user-image" src="/api/images/fixture-image.png" alt="Sample screenshot" loading="lazy" decoding="async"/></button></p></div>',
  )
})

it('uses the server source for a copied assistant image', () => {
  expect(
    renderMessage('![](/tmp/fixture.png)', undefined, {
      '/tmp/fixture.png':
        '/api/sessions/fixture-session/answer-images/fixture-key',
    }),
  ).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge image"><img class="chat-user-image" src="/api/sessions/fixture-session/answer-images/fixture-key" alt="" loading="lazy" decoding="async"/></button></p></div>',
  )
})

it('matches copied assistant images with spaces and non-ASCII path characters', () => {
  const path = '/tmp/画像 samples/final image.png'
  expect(
    renderMessage(`![](<${path}>)`, undefined, {
      [path]: '/api/sessions/fixture-session/answer-images/fixture-key',
    }),
  ).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge image"><img class="chat-user-image" src="/api/sessions/fixture-session/answer-images/fixture-key" alt="" loading="lazy" decoding="async"/></button></p></div>',
  )
})

const escapedPath = String.raw`/tmp/fixture\(v1\).png`

it.each([
  [
    'a Markdown escape is not mapped',
    `![](<${escapedPath}>)`,
    '/tmp/fixture(v1).png',
  ],
  [
    'a reference image is not mapped',
    '![fixture][sample-image]\n\n[sample-image]: </tmp/fixture.png>',
    '/tmp/fixture.png',
  ],
])('shows unavailable local images when %s', (_name, markdown, src) => {
  expect(renderMessage(markdown, undefined, {})).toBe(
    `<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: ${src}</span></p></div>`,
  )
})

it('shows an unavailable message for an ineligible assistant image', () => {
  expect(
    renderMessage('![](/tmp/fixture.txt)', undefined, {
      '/tmp/fixture.txt': null,
    }),
  ).toBe(
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/fixture.txt</span></p></div>',
  )
})

it.each([
  [
    'a matched unavailable path',
    { '/tmp/画像 samples/final image.png': null },
    '/tmp/画像 samples/final image.png',
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/画像 samples/final image.png</span></p></div>',
  ],
  [
    'an unmatched path',
    {},
    '/tmp/画像 samples/final image.png',
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/画像 samples/final image.png</span></p></div>',
  ],
  [
    'a matched path containing a literal percent sequence',
    { '/tmp/fixture%20name.png': null },
    '/tmp/fixture%20name.png',
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/fixture%20name.png</span></p></div>',
  ],
])('shows the original path for %s', (_name, imageSources, path, expected) => {
  expect(renderMessage(`![](<${path}>)`, undefined, imageSources)).toBe(
    expected,
  )
})

it('keeps untagged fenced code in a preformatted block', () => {
  const html = renderMessage(`~~~
set VALUE=preview
~~~`)

  expect(html).toBe(
    '<div class="markdown-message"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">set VALUE=preview\n</code></pre></div></div>',
  )
})

it('shows a copy toolbar for every fenced code block', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage>{'~~~text\nsample\n~~~'}</MarkdownMessage>,
  )

  expect(html).toBe(
    '<div class="markdown-message"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">sample\n</code></pre></div></div>',
  )
})

it('keeps raw HTML as text and blocks unsafe link protocols', () => {
  const html = renderMessage(
    '<img src=x onerror=alert(1)> <script>alert(1)</script> [unsafe](javascript:alert(1))',
  )

  expect(html).toBe(
    '<div class="markdown-message"><p>&lt;img src=x onerror=alert(1)&gt; &lt;script&gt;alert(1)&lt;/script&gt; <a href="" target="_blank" rel="noopener noreferrer">unsafe</a></p></div>',
  )
})

it('renders details and summary while keeping other raw HTML as text', () => {
  const html = renderMessage(`<details open>
<summary>Additional information</summary>

Body content is parsed as Markdown. http://example.com/日本語

<img src=x onerror=alert(1)>

<script>alert(1)</script>
</details>`)

  expect(html).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Additional information</span></button><div class="markdown-details-body"><p>Body content is parsed as Markdown. <a href="http://example.com/" target="_blank" rel="noopener noreferrer">http://example.com/</a>日本語</p>&lt;img src=x onerror=alert(1)&gt;&lt;script&gt;alert(1)&lt;/script&gt;</div></div></div>',
  )
})

it('renders an inline summary on the details opening line', () => {
  const html = renderMessage(
    '<details open><summary>Inline summary</summary>\nBody content\n</details>',
  )

  expect(html).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Inline summary</span></button><div class="markdown-details-body"><p>Body content</p></div></div></div>',
  )
})

it.each([
  {
    name: 'a list item',
    markdown: `- Before

  <details open>
  <summary>List details</summary>

  List body
  </details>`,
    expected:
      '<div class="markdown-message"><ul><li><p>Before</p><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">List details</span></button><div class="markdown-details-body"><p>List body</p></div></div></li></ul></div>',
  },
  {
    name: 'a blockquote',
    markdown: `> Before
>
> <details open><summary>Quoted details</summary>
>
> Quoted body
> </details>`,
    expected:
      '<div class="markdown-message"><blockquote><p>Before</p><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Quoted details</span></button><div class="markdown-details-body"><p>Quoted body</p></div></div></blockquote></div>',
  },
])('renders details contained in $name', ({ markdown, expected }) => {
  expect(renderMessage(markdown)).toBe(expected)
})

it('keeps details that close outside their list item as text once', () => {
  expect(
    renderMessage(`- Before
  <details>
  <summary>Split details</summary>
  Body

</details>`),
  ).toBe(
    '<div class="markdown-message"><ul><li><p>Before</p>&lt;details&gt;\n&lt;summary&gt;Split details&lt;/summary&gt;\nBody</li></ul>&lt;/details&gt;</div>',
  )
})

it('keeps details as text when content precedes the summary', () => {
  expect(
    renderMessage(`<details>
foo
<summary>Summary</summary>

Body
</details>`),
  ).toBe(
    '<div class="markdown-message">&lt;details&gt;<p>foo</p>&lt;summary&gt;Summary&lt;/summary&gt;<p>Body</p>&lt;/details&gt;</div>',
  )
})

it('keeps unsafe summary HTML as text', () => {
  expect(
    renderMessage(
      '<details open><summary><img src=x onerror=alert(1)></summary>\n\nBody\n</details>',
    ),
  ).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">&lt;img src=x onerror=alert(1)&gt;</span></button><div class="markdown-details-body"><p>Body</p></div></div></div>',
  )
})

it('renders nested details with both opening tags on the same line', () => {
  expect(
    renderMessage(`<details open><summary>Outer</summary><details open><summary>Inner</summary>
Inner body
</details></details>`),
  ).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Outer</span></button><div class="markdown-details-body"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Inner</span></button><div class="markdown-details-body"><p>Inner body</p></div></div></div></div></div>',
  )
})

it.each([
  {
    name: 'a heading',
    markdown: `<details>
<summary>Context</summary>

Hidden context
</details>
## Next heading`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="false" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Context</span></button></div><h2>Next heading</h2></div>',
  },
  {
    name: 'inline text',
    markdown: `<details open>
<summary>Context</summary>
Hidden context
</details>Visible text`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Context</span></button><div class="markdown-details-body"><p>Hidden context</p></div></div><p>Visible text</p></div>',
  },
  {
    name: 'a non-ASCII autolink',
    markdown: `<details open>
<summary>Context</summary>
Hidden context
</details>http://example.com/日本語 説明`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Context</span></button><div class="markdown-details-body"><p>Hidden context</p></div></div><p><a href="http://example.com/" target="_blank" rel="noopener noreferrer">http://example.com/</a>日本語 説明</p></div>',
  },
  {
    name: 'a second details block',
    markdown: `<details>
<summary>First section</summary>
First body
</details>
<details>
<summary>Second section</summary>
Second body
</details>`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="false" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">First section</span></button></div><div class="markdown-details"><button aria-expanded="false" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Second section</span></button></div></div>',
  },
])('preserves $name after details', ({ markdown, expected }) => {
  expect(renderMessage(markdown)).toBe(expected)
})

it.each([
  {
    name: 'a reference defined outside the block',
    markdown: `<details open>
<summary>References</summary>

See the [guide][shared-guide].
</details>

[shared-guide]: https://example.com/guide`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">References</span></button><div class="markdown-details-body"><p>See the <a href="https://example.com/guide" target="_blank" rel="noopener noreferrer">guide</a>.</p></div></div></div>',
  },
  {
    name: 'source order for duplicate definitions',
    markdown: `[shared-guide]: https://example.com/outside

<details open>
<summary>References</summary>

See the [guide][shared-guide].
[shared-guide]: https://example.com/inside
</details>`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">References</span></button><div class="markdown-details-body"><p>See the <a href="https://example.com/outside" target="_blank" rel="noopener noreferrer">guide</a>.</p></div></div></div>',
  },
  {
    name: 'a summary reference defined in the body',
    markdown: `<details open>
<summary>See the [guide][shared-guide]</summary>

[shared-guide]: https://example.com/summary

Body
</details>`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">See the <a href="https://example.com/summary" target="_blank" rel="noopener noreferrer">guide</a></span></button><div class="markdown-details-body"><p>Body</p></div></div></div>',
  },
  {
    name: 'an outer-body reference inside nested details',
    markdown: `<details open>
<summary>Outer</summary>

[shared-guide]: https://example.com/nested

<details open>
<summary>Inner</summary>

See the [guide][shared-guide].
</details>
</details>`,
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Outer</span></button><div class="markdown-details-body"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Inner</span></button><div class="markdown-details-body"><p>See the <a href="https://example.com/nested" target="_blank" rel="noopener noreferrer">guide</a>.</p></div></div></div></div></div>',
  },
])('resolves $name', ({ markdown, expected }) => {
  expect(renderMessage(markdown)).toBe(expected)
})

it('preserves indentation in a leading code block inside details', () => {
  expect(
    renderMessage(`<details open>
<summary>Command</summary>

    cargo build --locked
</details>`),
  ).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Command</span></button><div class="markdown-details-body"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">cargo build --locked\n</code></pre></div></div></div></div>',
  )
})

it('preserves all paragraphs in a details summary', () => {
  expect(
    renderMessage(`<details>
<summary>
First sentence.

Second sentence.
</summary>
</details>`),
  ).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="false" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">First sentence.<br/>Second sentence.</span></button></div></div>',
  )
})

it.each([
  {
    name: 'the details tag is unclosed',
    markdown: '<details>\n<summary>Unclosed details</summary>\nBody',
    expected:
      '<div class="markdown-message">&lt;details&gt;\n&lt;summary&gt;Unclosed details&lt;/summary&gt;\nBody</div>',
  },
  {
    name: 'the details tag has an onclick attribute',
    markdown:
      '<details onclick="runUnsafe()">\n<summary>Unsafe details</summary>\nBody\n</details>',
    expected:
      '<div class="markdown-message">&lt;details onclick=&quot;runUnsafe()&quot;&gt;\n&lt;summary&gt;Unsafe details&lt;/summary&gt;\nBody\n&lt;/details&gt;</div>',
  },
  {
    name: 'the open details tag has an onclick attribute',
    markdown:
      '<details open onclick="runUnsafe()">\n<summary>Unsafe details</summary>\nBody\n</details>',
    expected:
      '<div class="markdown-message">&lt;details open onclick=&quot;runUnsafe()&quot;&gt;\n&lt;summary&gt;Unsafe details&lt;/summary&gt;\nBody\n&lt;/details&gt;</div>',
  },
  {
    name: 'the summary tag has a style attribute',
    markdown:
      '<details>\n<summary style="color: red">Styled summary</summary>\nBody\n</details>',
    expected:
      '<div class="markdown-message">&lt;details&gt;\n&lt;summary style=&quot;color: red&quot;&gt;Styled summary&lt;/summary&gt;\nBody\n&lt;/details&gt;</div>',
  },
])(
  'keeps unsupported or incomplete details HTML as text when $name',
  ({ markdown, expected }) => {
    expect(renderMessage(markdown)).toBe(expected)
  },
)

it('keeps closing details tags inside fenced and indented code blocks', () => {
  const html = renderMessage(`<details open>
<summary>Code samples</summary>

~~~
</details>
~~~

    </details>
    Still indented.

Still inside.
</details>`)

  expect(html).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Code samples</span></button><div class="markdown-details-body"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">&lt;/details&gt;\n</code></pre></div><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">&lt;/details&gt;\nStill indented.\n</code></pre></div><p>Still inside.</p></div></div></div>',
  )
})

it.each([
  {
    name: 'an inline code span',
    markdown:
      '<details open><summary>Code</summary>\n\n`</details>`\nBody\n</details>',
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Code</span></button><div class="markdown-details-body"><p><code>&lt;/details&gt;</code></p><p>Body</p></div></div></div>',
  },
  {
    name: 'a multiline code span interrupted by an HTML block',
    markdown:
      '<details open><summary>Code</summary>\n`\n</details>\n`\n\nBody\n</details>',
    expected:
      '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Code</span></button><div class="markdown-details-body"><p>`</p></div></div><p>`</p><p>Body</p>&lt;/details&gt;</div>',
  },
])(
  'handles closing tags in $name according to CommonMark',
  ({ markdown, expected }) => {
    expect(renderMessage(markdown)).toBe(expected)
  },
)

it.each([
  {
    name: 'script',
    source: `<script>\n</details>\n</script>`,
    escaped: '&lt;script&gt;\n&lt;/details&gt;\n&lt;/script&gt;',
  },
  {
    name: 'style',
    source: `<style>\n.example { content: </details>; }\n</style>`,
    escaped:
      '&lt;style&gt;\n.example { content: &lt;/details&gt;; }\n&lt;/style&gt;',
  },
])(
  'keeps closing details tags inside $name raw text',
  ({ source, escaped }) => {
    expect(
      renderMessage(`<details>
<summary>Example</summary>

${source}

Hidden body content
</details>`),
    ).toBe(
      `<div class="markdown-message"><div class="markdown-details"><button aria-expanded="false" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Example</span></button><div class="markdown-details-body">${escaped}<p>Hidden body content</p></div></div></div>`,
    )
  },
)

it('keeps closing tags in HTML comments inside the details body', () => {
  expect(
    renderMessage(`<details open>
<summary>Comment</summary>

<!--
</details>
-->

Body
</details>`),
  ).toBe(
    '<div class="markdown-message"><div class="markdown-details"><button aria-expanded="true" class="markdown-details-summary" type="button"><svg aria-hidden="true" class="markdown-details-chevron" data-expanded="true" viewBox="0 0 10 10"><path d="M2 1l6 4-6 4z" fill="currentColor"></path></svg><span class="markdown-details-summary-content">Comment</span></button><div class="markdown-details-body">&lt;!--\n&lt;/details&gt;\n--&gt;<p>Body</p></div></div></div>',
  )
})

it('renders br tags as line breaks in paragraphs and table cells', () => {
  const html = renderMessage(
    `before<br>middle<BR/>after<br />last

| Content |
| --- |
| first<br>second<BR/>third<br />fourth |`,
  )

  expect(html).toBe(
    [
      '<div class="markdown-message"><p>before<br/>',
      'middle<br/>',
      'after<br/>',
      'last</p>',
      '<table><thead><tr><th>Content</th></tr></thead><tbody><tr><td>first<br/>',
      'second<br/>',
      'third<br/>',
      'fourth</td></tr></tbody></table></div>',
    ].join(String.fromCharCode(10)),
  )
})

it('trims non-ASCII text from automatic links and preserves explicit links', () => {
  const html = renderMessage(
    'http://localhost:43127。承認後に処理する [http://localhost:43127。承認後に処理する](http://localhost:43127。承認後に処理する) and http://example.com and http://example.com/日本語',
  )

  expect(html).toBe(
    '<div class="markdown-message"><p><a href="http://localhost:43127" target="_blank" rel="noopener noreferrer">http://localhost:43127</a>。承認後に処理する <a href="http://localhost:43127%E3%80%82%E6%89%BF%E8%AA%8D%E5%BE%8C%E3%81%AB%E5%87%A6%E7%90%86%E3%81%99%E3%82%8B" target="_blank" rel="noopener noreferrer">http://localhost:43127。承認後に処理する</a> and <a href="http://example.com" target="_blank" rel="noopener noreferrer">http://example.com</a> and <a href="http://example.com/" target="_blank" rel="noopener noreferrer">http://example.com/</a>日本語</p></div>',
  )
})

it('adds a confirmation button to agent inline bang commands', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage sessionId="sample-session">
      {'Run `! printf safe`.'}
    </MarkdownMessage>,
  )

  expect(html).toBe(
    '<div class="markdown-message"><p>Run <span class="bang-command-inline"><code>! printf safe</code><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span></span></span>.</p></div>',
  )
})

it('adds per-line and run-all buttons to fenced bang commands', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage sessionId="sample-session">
      {'```sh\n! printf first\n! printf second\n```'}
    </MarkdownMessage>,
  )

  expect(html).toBe(
    [
      '<div class="markdown-message"><div class="markdown-code-block"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">sh</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="sh"><code class="language-sh">! printf first',
      '! printf second',
      '</code></pre></div><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span><span class="bang-command-control"><button aria-label="Run command 2" class="bang-command-button" type="button">Run command 2</button></span><span class="bang-command-control"><button aria-label="Run all" class="bang-command-button" type="button">Run all</button></span></span></div></div>',
    ].join(String.fromCharCode(10)),
  )
})

it('adds a confirmation button to an indented bang command block', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage sessionId="sample-session">
      {'    ! printf safe'}
    </MarkdownMessage>,
  )

  expect(html).toBe(
    '<div class="markdown-message"><div class="markdown-code-block"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">! printf safe\n</code></pre></div><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span></span></div></div>',
  )
})
