import { useEffect } from 'react';
import { useReview } from './store.ts';
import { fileAnchorId } from './components/FileCard.tsx';

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable;
}

function scrollToFile(path: string): void {
  document.getElementById(fileAnchorId(path))?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

function stepHunk(container: HTMLElement | null, direction: 1 | -1): void {
  if (!container) return;

  const headers = [...container.querySelectorAll<HTMLElement>('[data-hunk]')];
  if (headers.length === 0) return;

  const top = container.getBoundingClientRect().top;
  const offsets = headers.map((h) => h.getBoundingClientRect().top - top);

  const target =
    direction === 1
      ? headers[offsets.findIndex((o) => o > 8)]
      : [...headers].reverse()[[...offsets].reverse().findIndex((o) => o < -8)];

  target?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

export function useKeyboard(scrollRef: React.RefObject<HTMLDivElement | null>): void {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const store = useReview.getState();

      if (e.key === 'Escape') {
        if (store.newSessionOpen) store.setNewSessionOpen(false);
        else if (store.pickerOpen) store.setPickerOpen(false);
        else if (store.shortcutsOpen) store.toggleShortcuts();
        else if (store.preview !== null) store.closePreview();
        else if (store.composingPath) store.setComposingPath(null);
        else store.clearSelection();
        return;
      }

      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !isTyping(e.target)) {
        e.preventDefault();
        void store.openPreview();
        return;
      }

      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (store.newSessionOpen) return;

      if (e.key === 's') {
        e.preventDefault();
        if (store.sessionId) store.setPickerOpen(!store.pickerOpen);
        return;
      }
      if (!store.sessionId) return;

      const { model, activePath } = store;
      const files = model?.files ?? [];
      const at = files.findIndex((f) => f.path === activePath);

      switch (e.key) {
        case '?':
          store.toggleShortcuts();
          break;
        case 'b':
          store.toggleSidebar();
          break;
        case 'w':
          store.toggleWrap();
          break;
        case 't':
          store.setViewMode(store.viewMode === 'unified' ? 'split' : 'unified');
          break;
        case 'r':
          void store.load();
          break;
        case 'j':
          stepHunk(scrollRef.current, 1);
          break;
        case 'k':
          stepHunk(scrollRef.current, -1);
          break;
        case 'n': {
          const next = files[Math.min(at + 1, files.length - 1)];
          if (next) {
            store.setActivePath(next.path);
            scrollToFile(next.path);
          }
          break;
        }
        case 'p': {
          const previous = files[Math.max(at - 1, 0)];
          if (previous) {
            store.setActivePath(previous.path);
            scrollToFile(previous.path);
          }
          break;
        }
        case 'c':
          if (activePath) store.setComposingPath(activePath);
          break;
        case 'v': {
          const file = files[at];
          if (file) void store.setViewed(file, store.draft?.viewed[file.path] !== file.digest);
          break;
        }
        default:
          return;
      }

      e.preventDefault();
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [scrollRef]);
}
