import { useEffect, useRef } from 'react';
import { useReview } from '../store.ts';
import { SessionRow } from './SessionRow.tsx';

/** The dropdown under the header's session button. */
export function SessionPicker() {
  const open = useReview((s) => s.pickerOpen);
  const setOpen = useReview((s) => s.setPickerOpen);
  const sessions = useReview((s) => s.sessions);
  const sessionId = useReview((s) => s.sessionId);
  const selectSession = useReview((s) => s.selectSession);
  const setNewSessionOpen = useReview((s) => s.setNewSessionOpen);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Element | null;
      if (target?.closest('[data-session-button]')) return;
      if (ref.current && !ref.current.contains(target)) setOpen(false);
    };
    window.addEventListener('mousedown', onDown);
    return () => window.removeEventListener('mousedown', onDown);
  }, [open, setOpen]);

  if (!open) return null;

  return (
    <div
      ref={ref}
      className="absolute top-full left-0 z-40 mt-1 max-h-[70vh] w-[26rem] overflow-y-auto rounded-lg border shadow-lg"
      style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
    >
      <div className="flex items-center border-b px-3 py-2" style={{ borderColor: 'var(--border-subtle)' }}>
        <span className="text-[11px] font-medium" style={{ color: 'var(--text-muted)' }}>
          Sessions
        </span>
        <button
          type="button"
          onClick={() => selectSession(null)}
          className="ml-auto cursor-pointer text-[11px]"
          style={{ color: 'var(--text-muted)' }}
        >
          All sessions
        </button>
        <button
          type="button"
          onClick={() => setNewSessionOpen(true)}
          className="ml-3 cursor-pointer rounded px-2 py-0.5 text-[11px] font-medium text-white"
          style={{ background: 'var(--accent)' }}
        >
          New session
        </button>
      </div>

      {(sessions ?? []).map((session) => (
        <SessionRow
          key={session.id}
          session={session}
          active={session.id === sessionId}
          onSelect={() => selectSession(session.id)}
        />
      ))}

      {sessions?.length === 0 && (
        <p className="px-3 py-4" style={{ color: 'var(--text-muted)' }}>
          No sessions yet.
        </p>
      )}
    </div>
  );
}
