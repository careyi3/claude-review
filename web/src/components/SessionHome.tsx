import { useReview } from '../store.ts';
import { SessionRow } from './SessionRow.tsx';

/** Shown when no session is selected: every session, and a way to add one. */
export function SessionHome() {
  const sessions = useReview((s) => s.sessions);
  const sessionsError = useReview((s) => s.sessionsError);
  const selectSession = useReview((s) => s.selectSession);
  const setNewSessionOpen = useReview((s) => s.setNewSessionOpen);

  const waiting = (sessions ?? []).filter((s) => s.status === 'submitted').length;

  return (
    <div className="mx-auto w-full max-w-2xl p-8">
      <div className="mb-4 flex items-center gap-3">
        <h1 className="text-base font-semibold">claude-review</h1>
        {waiting > 0 && (
          <span className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
            {waiting} waiting for Claude
          </span>
        )}
        <button
          type="button"
          onClick={() => setNewSessionOpen(true)}
          className="ml-auto cursor-pointer rounded px-3 py-1.5 text-[11px] font-medium text-white"
          style={{ background: 'var(--accent)' }}
        >
          New session
        </button>
      </div>

      {sessionsError && <p className="mb-3 text-rose-600 dark:text-rose-400">{sessionsError}</p>}

      {sessions === null ? (
        <p style={{ color: 'var(--text-muted)' }}>Loading sessions…</p>
      ) : sessions.length === 0 ? (
        <div
          className="rounded-lg border p-6"
          style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
        >
          <p className="mb-3 font-medium">No sessions yet</p>
          <p className="mb-2" style={{ color: 'var(--text-muted)' }}>
            A session is a worktree to review. Make one any of these ways:
          </p>
          <ul className="list-disc space-y-1 pl-5" style={{ color: 'var(--text-muted)' }}>
            <li>
              Run <code className="mono">/review-start</code> in Claude Code.
            </li>
            <li>
              Run <code className="mono">claude-review</code> inside a repository.
            </li>
            <li>Press New session and pick a directory.</li>
          </ul>
        </div>
      ) : (
        <div
          className="divide-y overflow-hidden rounded-lg border"
          style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
        >
          {sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              active={false}
              onSelect={() => selectSession(session.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
