import { useReview } from '../store.ts';

const SHORTCUTS: [string, string][] = [
  ['s', 'Switch session'],
  ['j / k', 'Next / previous hunk'],
  ['n / p', 'Next / previous file'],
  ['c', 'Comment on the current file'],
  ['v', 'Mark the current file viewed'],
  ['t', 'Toggle unified and split'],
  ['w', 'Wrap or scroll long lines'],
  ['b', 'Show or hide the file list'],
  ['r', 'Reload the diff'],
  ['⌘↵', 'Submit the review'],
  ['esc', 'Close, or clear the selection'],
  ['?', 'This list'],
];

export function ShortcutsOverlay() {
  const open = useReview((s) => s.shortcutsOpen);
  const toggle = useReview((s) => s.toggleShortcuts);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-8"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={toggle}
    >
      <div
        className="w-full max-w-sm rounded-lg border p-4"
        style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-3 font-medium">Keyboard shortcuts</h2>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
          {SHORTCUTS.map(([keys, description]) => (
            <div key={keys} className="contents">
              <dt className="mono text-right">{keys}</dt>
              <dd style={{ color: 'var(--text-muted)' }}>{description}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}
