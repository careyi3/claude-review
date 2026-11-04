import { useEffect, useRef } from 'react';
import { useReview } from './store.ts';
import { Header } from './components/Header.tsx';
import { FileList } from './components/FileList.tsx';
import { FileCard } from './components/FileCard.tsx';
import { OverallPanel } from './components/OverallPanel.tsx';
import { StalePanel } from './components/StalePanel.tsx';
import { SubmitDialog } from './components/SubmitDialog.tsx';
import { SubmittedBanner } from './components/SubmittedBanner.tsx';
import { ShortcutsOverlay } from './components/ShortcutsOverlay.tsx';
import { StaleBanner } from './components/StaleBanner.tsx';
import { NewSessionDialog } from './components/NewSessionDialog.tsx';
import { SessionHome } from './components/SessionHome.tsx';
import { useKeyboard } from './useKeyboard.ts';
import { useLiveReload } from './useLiveReload.ts';

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-full items-center justify-center p-8 text-center">
      <div className="max-w-md">{children}</div>
    </div>
  );
}

export function App() {
  const { model, draft, status, error, load, setActivePath } = useReview();
  const endSelection = useReview((s) => s.endSelection);
  const clearSelection = useReview((s) => s.clearSelection);
  const sidebarOpen = useReview((s) => s.sidebarOpen);
  const sessionId = useReview((s) => s.sessionId);
  const loadSessions = useReview((s) => s.loadSessions);
  const selectSession = useReview((s) => s.selectSession);
  const scrollRef = useRef<HTMLDivElement>(null);

  useKeyboard(scrollRef);
  useLiveReload();

  useEffect(() => {
    void loadSessions();
    void load();
  }, [load, loadSessions]);

  useEffect(() => {
    const onMouseUp = () => endSelection();
    window.addEventListener('mouseup', onMouseUp);
    return () => window.removeEventListener('mouseup', onMouseUp);
  }, [endSelection]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container || !model) return;

    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
        const path = visible?.target.getAttribute('data-path');
        if (path) setActivePath(path);
      },
      { root: container, rootMargin: '0px 0px -70% 0px' },
    );

    for (const el of container.querySelectorAll('[data-path]')) observer.observe(el);
    return () => observer.disconnect();
  }, [model, setActivePath]);

  if (!sessionId) {
    return (
      <div className="h-full overflow-y-auto">
        <NewSessionDialog />
        <SessionHome />
      </div>
    );
  }

  const backToSessions = (
    <button
      type="button"
      onClick={() => selectSession(null)}
      className="cursor-pointer rounded border px-3 py-1.5"
      style={{ borderColor: 'var(--border)' }}
    >
      All sessions
    </button>
  );

  if (status === 'gone') {
    return (
      <Centered>
        <p className="mb-3 font-medium">This session’s worktree is gone</p>
        <p className="mb-4" style={{ color: 'var(--text-muted)' }}>
          {error} It was probably removed after the work was merged. Close the session from the
          list, or recreate the worktree.
        </p>
        {backToSessions}
      </Centered>
    );
  }

  if (status === 'error') {
    return (
      <Centered>
        <p className="mb-3 font-medium text-rose-600 dark:text-rose-400">Could not load the diff</p>
        <p className="mb-4" style={{ color: 'var(--text-muted)' }}>
          {error}
        </p>
        <button
          type="button"
          onClick={() => void load()}
          className="cursor-pointer rounded border px-3 py-1.5"
          style={{ borderColor: 'var(--border)' }}
        >
          Try again
        </button>{' '}
        {backToSessions}
      </Centered>
    );
  }

  if (!model || !draft) {
    return (
      <Centered>
        <p style={{ color: 'var(--text-muted)' }}>Loading diff…</p>
      </Centered>
    );
  }

  const knownPaths = new Set(model.files.map((f) => f.path));
  const stale = draft.comments.filter((c) => !knownPaths.has(c.path));

  return (
    <div className="flex h-full flex-col">
      <SubmitDialog />
      <ShortcutsOverlay />
      <NewSessionDialog />
      <Header model={model} draft={draft} onReload={() => void load()} reloading={status === 'loading'} />

      <StaleBanner />

      <div className="flex min-h-0 flex-1">
        {sidebarOpen && (
          <aside
            className="w-72 shrink-0 overflow-y-auto border-r"
            style={{ borderColor: 'var(--border)', background: 'var(--bg-raised)' }}
          >
            <FileList model={model} draft={draft} />
          </aside>
        )}

        <main ref={scrollRef} className="min-w-0 flex-1 overflow-y-auto p-4">
          <SubmittedBanner />

          {model.files.length === 0 && stale.length === 0 ? (
            <Centered>
              <p style={{ color: 'var(--text-muted)' }}>
                The working tree is clean — nothing to review against {model.base.rev}.
              </p>
            </Centered>
          ) : (
            <>
              <OverallPanel draft={draft} />
              <StalePanel comments={stale} />
              {model.files.map((file) => (
                <FileCard key={file.path} file={file} />
              ))}
            </>
          )}
        </main>
      </div>
    </div>
  );
}
