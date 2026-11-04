import { useEffect, useState } from 'react';
import * as api from '../api.ts';
import type { Suggestion } from '../types.ts';
import { tidyPath } from '../format.ts';
import { useReview } from '../store.ts';

type BaseChoice = 'auto' | 'HEAD' | 'custom';

const SOURCE_LABEL: Record<Suggestion['source'], string> = {
  claude: 'recent Claude project',
  worktree: 'worktree',
};

export function NewSessionDialog() {
  const open = useReview((s) => s.newSessionOpen);
  const setOpen = useReview((s) => s.setNewSessionOpen);
  const selectSession = useReview((s) => s.selectSession);
  const loadSessions = useReview((s) => s.loadSessions);

  const [path, setPath] = useState('');
  const [name, setName] = useState('');
  const [baseChoice, setBaseChoice] = useState<BaseChoice>('auto');
  const [customBase, setCustomBase] = useState('');
  const [suggestions, setSuggestions] = useState<Suggestion[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    if (!open) return;
    setPath('');
    setName('');
    setBaseChoice('auto');
    setCustomBase('');
    setError(null);
    setSuggestions(null);
    api
      .fetchSuggestions()
      .then(setSuggestions)
      .catch(() => setSuggestions([]));
  }, [open]);

  if (!open) return null;

  const base = baseChoice === 'custom' ? customBase.trim() : baseChoice;
  const canCreate = path.trim() !== '' && base !== '' && !creating;

  const create = async () => {
    if (!canCreate) return;
    setCreating(true);
    setError(null);
    try {
      const created = await api.createSession({
        path: path.trim(),
        name: name.trim() || undefined,
        base,
      });
      setOpen(false);
      await loadSessions();
      selectSession(created.session.id);
      if (created.notes.length > 0) window.alert(created.notes.join('\n\n'));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setCreating(false);
    }
  };

  const filtered = (suggestions ?? []).filter(
    (s) => !path.trim() || s.path.toLowerCase().includes(path.trim().toLowerCase()),
  );

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-8"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={() => setOpen(false)}
    >
      <form
        className="flex max-h-full w-full max-w-xl flex-col overflow-hidden rounded-lg border"
        style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <header className="border-b px-4 py-3" style={{ borderColor: 'var(--border-subtle)' }}>
          <h2 className="font-medium">New session</h2>
          <p className="mt-1 text-[11px]" style={{ color: 'var(--text-muted)' }}>
            Point a session at a repository or worktree. If it already has an open session, that
            one is reused.
          </p>
        </header>

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-3">
          <label className="block">
            <span className="mb-1 block text-[11px] font-medium">Path</span>
            <input
              autoFocus
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="~/code/my-repo"
              className="mono w-full rounded border px-2 py-1.5"
              style={{ borderColor: 'var(--border)', background: 'var(--bg)' }}
            />
          </label>

          {suggestions === null ? (
            <p className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
              Looking for recent projects…
            </p>
          ) : (
            filtered.length > 0 && (
              <div
                className="max-h-48 overflow-y-auto rounded border"
                style={{ borderColor: 'var(--border-subtle)' }}
              >
                {filtered.map((s) => (
                  <button
                    key={s.path}
                    type="button"
                    onClick={() => setPath(s.path)}
                    className="flex w-full cursor-pointer items-baseline gap-2 px-2 py-1 text-left"
                    style={{ background: s.path === path ? 'var(--bg-sunken)' : 'transparent' }}
                  >
                    <span className="mono min-w-0 flex-1 truncate" title={s.path}>
                      {tidyPath(s.path)}
                    </span>
                    {s.branch && (
                      <span className="mono shrink-0 text-[10px]" style={{ color: 'var(--text-muted)' }}>
                        {s.branch}
                      </span>
                    )}
                    <span className="shrink-0 text-[10px]" style={{ color: 'var(--text-muted)' }}>
                      {SOURCE_LABEL[s.source]}
                    </span>
                  </button>
                ))}
              </div>
            )
          )}

          <label className="block">
            <span className="mb-1 block text-[11px] font-medium">Name (optional)</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="defaults to folder · branch"
              className="w-full rounded border px-2 py-1.5"
              style={{ borderColor: 'var(--border)', background: 'var(--bg)' }}
            />
          </label>

          <fieldset>
            <legend className="mb-1 text-[11px] font-medium">Compare against</legend>
            <div className="space-y-1">
              <label className="flex items-center gap-2">
                <input type="radio" checked={baseChoice === 'auto'} onChange={() => setBaseChoice('auto')} />
                <span>Everything on the branch</span>
                <span className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                  merge-base with main, committed or not
                </span>
              </label>
              <label className="flex items-center gap-2">
                <input type="radio" checked={baseChoice === 'HEAD'} onChange={() => setBaseChoice('HEAD')} />
                <span>Uncommitted changes only</span>
                <span className="mono text-[11px]" style={{ color: 'var(--text-muted)' }}>
                  HEAD
                </span>
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  checked={baseChoice === 'custom'}
                  onChange={() => setBaseChoice('custom')}
                />
                <span>A revision</span>
                <input
                  value={customBase}
                  onChange={(e) => {
                    setCustomBase(e.target.value);
                    setBaseChoice('custom');
                  }}
                  placeholder="origin/main, HEAD~3, a1b2c3d"
                  className="mono flex-1 rounded border px-2 py-0.5 text-[11px]"
                  style={{ borderColor: 'var(--border)', background: 'var(--bg)' }}
                />
              </label>
            </div>
          </fieldset>

          {error && <p className="text-rose-600 dark:text-rose-400">{error}</p>}
        </div>

        <footer
          className="flex items-center gap-2 border-t px-4 py-3"
          style={{ borderColor: 'var(--border-subtle)' }}
        >
          <button
            type="submit"
            disabled={!canCreate}
            className="cursor-pointer rounded px-3 py-1.5 text-[11px] font-medium text-white disabled:cursor-not-allowed disabled:opacity-40"
            style={{ background: 'var(--accent)' }}
          >
            {creating ? 'Creating…' : 'Create session'}
          </button>
          <button
            type="button"
            onClick={() => setOpen(false)}
            className="cursor-pointer rounded border px-3 py-1.5 text-[11px]"
            style={{ borderColor: 'var(--border)' }}
          >
            Cancel
          </button>
        </footer>
      </form>
    </div>
  );
}
