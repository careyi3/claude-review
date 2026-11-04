import type { SessionSummary } from '../types.ts';
import { timeAgo, tidyPath } from '../format.ts';
import { useReview } from '../store.ts';

export function StatusBadge({ session }: { session: SessionSummary }) {
  if (session.missing) {
    return (
      <span className="rounded px-1.5 py-px text-[10px] text-white" style={{ background: '#b4231f' }}>
        worktree gone
      </span>
    );
  }
  if (session.status === 'submitted') {
    return (
      <span
        className="rounded px-1.5 py-px text-[10px] text-white"
        style={{ background: 'var(--accent)' }}
        title="Submitted; run /review-feedback in Claude Code"
      >
        waiting for Claude
      </span>
    );
  }
  return null;
}

interface Props {
  session: SessionSummary;
  active: boolean;
  onSelect: () => void;
}

export function SessionRow({ session, active, onSelect }: Props) {
  const closeSession = useReview((s) => s.closeSession);
  const renameSession = useReview((s) => s.renameSession);

  const when =
    session.status === 'submitted' ? `submitted ${timeAgo(session.submittedAt)}` : timeAgo(session.updatedAt);

  return (
    <div
      className="group flex items-start gap-2 px-3 py-2"
      style={{ background: active ? 'var(--bg-sunken)' : 'transparent' }}
    >
      <button type="button" onClick={onSelect} className="min-w-0 flex-1 cursor-pointer text-left">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium">{session.name}</span>
          <StatusBadge session={session} />
          {session.createdBy === 'claude' && (
            <span className="text-[10px]" style={{ color: 'var(--text-muted)' }} title="Registered by Claude">
              ✦
            </span>
          )}
        </div>
        <div className="mono truncate text-[11px]" style={{ color: 'var(--text-muted)' }} title={session.path}>
          {session.branch ?? 'detached'} · {tidyPath(session.path)}
        </div>
        <div className="text-[10px]" style={{ color: 'var(--text-muted)' }}>
          <span className="mono">{session.id}</span>
          {session.draftComments > 0 &&
            ` · ${session.draftComments} draft comment${session.draftComments === 1 ? '' : 's'}`}
          {session.rounds > 0 && ` · round ${session.rounds + (session.status === 'submitted' ? 0 : 1)}`}
          {when && ` · ${when}`}
        </div>
      </button>

      <div className="flex shrink-0 gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <button
          type="button"
          title="Rename"
          aria-label={`Rename ${session.name}`}
          className="cursor-pointer rounded px-1 text-[11px]"
          style={{ color: 'var(--text-muted)' }}
          onClick={() => {
            const name = window.prompt('Session name', session.name)?.trim();
            if (name && name !== session.name) void renameSession(session.id, name);
          }}
        >
          ✎
        </button>
        <button
          type="button"
          title="Close this session"
          aria-label={`Close ${session.name}`}
          className="cursor-pointer rounded px-1 text-[11px]"
          style={{ color: 'var(--text-muted)' }}
          onClick={() => {
            if (window.confirm(`Close “${session.name}”? Its draft and reviews stay on disk.`)) {
              void closeSession(session.id);
            }
          }}
        >
          ✕
        </button>
      </div>
    </div>
  );
}
