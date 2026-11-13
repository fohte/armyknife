import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { ReviewDocumentPanel } from '#components/review-document-panel'

it('renders typed frontmatter and the markdown body', () => {
  const markup = renderToStaticMarkup(
    <ReviewDocumentPanel
      frontmatter={{
        title: 'Fixture title',
        enabled: true,
        retries: 2,
        empty: null,
        labels: ['alpha', 'beta'],
        settings: { ready: false },
      }}
      body={`## Summary

Fixture body.`}
    />,
  )

  expect(markup).toBe(
    '<article class="review-document-panel"><section class="review-document-frontmatter" aria-label="Document metadata"><h2>Frontmatter</h2><dl class="review-document-fields"><div class="review-document-field"><dt class="review-document-field-key">title</dt><dd class="review-document-field-value"><span class="review-document-string">Fixture title</span></dd></div><div class="review-document-field"><dt class="review-document-field-key">enabled</dt><dd class="review-document-field-value"><span class="review-document-boolean" data-checked="true" aria-label="true">✓</span></dd></div><div class="review-document-field"><dt class="review-document-field-key">retries</dt><dd class="review-document-field-value"><span class="review-document-number">2</span></dd></div><div class="review-document-field"><dt class="review-document-field-key">empty</dt><dd class="review-document-field-value"><span class="review-document-null">null</span></dd></div><div class="review-document-field"><dt class="review-document-field-key">labels</dt><dd class="review-document-field-value"><ul class="review-document-array"><li><span class="review-document-string">alpha</span></li><li><span class="review-document-string">beta</span></li></ul></dd></div><div class="review-document-field-group"><dt class="review-document-group-title">settings</dt><dd class="review-document-group-content"><dl class="review-document-fields review-document-fields--nested"><div class="review-document-field"><dt class="review-document-field-key">ready</dt><dd class="review-document-field-value"><span class="review-document-boolean" data-checked="false" aria-label="false">−</span></dd></div></dl></dd></div></dl></section><div class="markdown-message review-document-body"><h2>Summary</h2>\n<p>Fixture body.</p></div></article>',
  )
})
