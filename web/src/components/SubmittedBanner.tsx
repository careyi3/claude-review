import { useReview } from '../store.ts';

export function SubmittedBanner() {
  const result = useReview((s) => s.submitResult);
  const dismiss = useReview((s) => s.dismissResult);
  const sessionId = useReview((s) => s.sessionId);

  if (!result) return null;

  return (
    <section
      className="mb-4 rounded-lg border p-3"
      style={{ borderColor: 'var(--accent)', background: 'var(--bg-raised)' }}
    >
      <div className="mb-2 flex items-center gap-2">
        <h2 className="font-medium">
          Review submitted — {result.commentCount} comment
          {result.commentCount === 1 ? '' : 's'}
        </h2>
        <button
          type="button"
          onClick={dismiss}
          className="ml-auto cursor-pointer text-[11px] opacity-60 hover:opacity-100"
        >
          Dismiss
        </button>
      </div>

      <p className="mb-2" style={{ color: 'var(--text-muted)' }}>
        Written to <code className="mono">{result.latestMarkdownPath}</code>
      </p>

      <p>
        Now run <code className="mono font-medium">/review-feedback</code> in Claude Code to have
        it work through your comments. If Claude did not start this session, pass the id:{' '}
        <code className="mono">/review-feedback {sessionId}</code>. Once it has made changes, review
        them here and submit another round.
      </p>
    </section>
  );
}
