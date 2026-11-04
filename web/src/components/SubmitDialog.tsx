import { useReview } from '../store.ts';

export function SubmitDialog() {
  const preview = useReview((s) => s.preview);
  const submitting = useReview((s) => s.submitting);
  const closePreview = useReview((s) => s.closePreview);
  const submit = useReview((s) => s.submit);

  if (preview === null) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-8"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={closePreview}
    >
      <div
        className="flex max-h-full w-full max-w-3xl flex-col overflow-hidden rounded-lg border"
        style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="border-b px-4 py-3" style={{ borderColor: 'var(--border-subtle)' }}>
          <h2 className="font-medium">Submit review</h2>
          <p className="mt-1 text-[11px]" style={{ color: 'var(--text-muted)' }}>
            Saves this as the session’s latest review, for <code>/review-feedback</code> to pick
            up, and clears the draft. A timestamped copy is kept beside it, so a mistaken submit can
            be recovered.
          </p>
        </header>

        <pre
          className="mono min-h-0 flex-1 overflow-auto whitespace-pre-wrap px-4 py-3"
          style={{ background: 'var(--bg)', overflowWrap: 'anywhere' }}
        >
          {preview}
        </pre>

        <footer
          className="flex items-center gap-2 border-t px-4 py-3"
          style={{ borderColor: 'var(--border-subtle)' }}
        >
          <button
            type="button"
            onClick={() => void submit()}
            disabled={submitting}
            className="cursor-pointer rounded px-3 py-1.5 text-[11px] font-medium text-white disabled:opacity-50"
            style={{ background: 'var(--accent)' }}
          >
            {submitting ? 'Writing…' : 'Write review'}
          </button>
          <button
            type="button"
            onClick={closePreview}
            className="cursor-pointer rounded border px-3 py-1.5 text-[11px]"
            style={{ borderColor: 'var(--border)' }}
          >
            Cancel
          </button>
        </footer>
      </div>
    </div>
  );
}
