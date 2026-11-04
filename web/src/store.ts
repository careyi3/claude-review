import { create } from 'zustand';
import type {
  CommentTag,
  Draft,
  FileDiff,
  ReviewModel,
  SessionSummary,
  Side,
} from './types.ts';
import { EXPAND_STEP, gapKey, type Expansion } from './gaps.ts';
import * as api from './api.ts';
import type { Selection } from './selection.ts';

export type { Selection } from './selection.ts';
export { selectionRange } from './selection.ts';

function initialViewMode(): ViewMode {
  return new URLSearchParams(window.location.search).get('view') === 'split' ? 'split' : 'unified';
}

function initialSession(): string | null {
  return new URLSearchParams(window.location.search).get('session');
}

/** Keeps `?session=` in the address bar so a reload reopens the same session. */
function rememberSessionInUrl(id: string | null): void {
  const url = new URL(window.location.href);
  if (id) url.searchParams.set('session', id);
  else url.searchParams.delete('session');
  window.history.replaceState(null, '', url);
}

const SIDEBAR_KEY = 'claude-review:sidebar';
const WRAP_KEY = 'claude-review:wrap';

function remembered(key: string, fallback: boolean, offValue: string): boolean {
  try {
    const stored = window.localStorage.getItem(key);
    return stored === null ? fallback : stored !== offValue;
  } catch {
    return fallback;
  }
}

function remember(key: string, value: boolean, onValue: string, offValue: string): void {
  try {
    window.localStorage.setItem(key, value ? onValue : offValue);
  } catch {
  }
}

type Status = 'loading' | 'ready' | 'error' | 'gone';
export type ViewMode = 'unified' | 'split';

interface ReviewState {
  sessions: SessionSummary[] | null;
  sessionsError: string | null;
  sessionId: string | null;
  pickerOpen: boolean;
  newSessionOpen: boolean;

  model: ReviewModel | null;
  draft: Draft | null;
  status: Status;
  error: string | null;
  saving: boolean;
  saveError: string | null;

  activePath: string | null;
  collapsed: ReadonlySet<string>;
  viewMode: ViewMode;
  selection: Selection | null;

  preview: string | null;
  submitting: boolean;
  submitResult: api.SubmitResult | null;

  sidebarOpen: boolean;
  wrap: boolean;
  shortcutsOpen: boolean;
  stale: boolean;
  expansions: Readonly<Record<string, Expansion>>;
  composingPath: string | null;

  loadSessions: () => Promise<void>;
  selectSession: (id: string | null) => void;
  setPickerOpen: (open: boolean) => void;
  setNewSessionOpen: (open: boolean) => void;
  closeSession: (id: string) => Promise<void>;
  renameSession: (id: string, name: string) => Promise<void>;
  setSessionBase: (id: string, base: string) => Promise<void>;

  load: () => Promise<void>;
  setActivePath: (path: string | null) => void;
  toggleCollapsed: (path: string) => void;
  setViewMode: (mode: ViewMode) => void;

  beginSelection: (path: string, side: Side, line: number) => void;
  extendSelection: (path: string, side: Side, line: number) => void;
  endSelection: () => void;
  clearSelection: () => void;

  setComposingPath: (path: string | null) => void;
  toggleSidebar: () => void;
  toggleWrap: () => void;
  toggleShortcuts: () => void;
  setStale: (stale: boolean) => void;
  expandGap: (path: string, index: number, edge: 'top' | 'bottom' | 'all', size: number) => void;
  setViewed: (file: FileDiff, viewed: boolean) => Promise<void>;

  openPreview: () => Promise<void>;
  closePreview: () => void;
  submit: () => Promise<void>;
  dismissResult: () => void;

  addComment: (input: api.NewCommentInput) => Promise<void>;
  editComment: (id: string, patch: { tag?: CommentTag; body?: string }) => Promise<void>;
  removeComment: (id: string) => Promise<void>;
  setOverall: (text: string) => Promise<void>;
}

const startingSession = initialSession();
api.setApiSession(startingSession);

/** Per-session view state, reset whenever the session changes. */
const FRESH_VIEW = {
  model: null,
  draft: null,
  error: null,
  saveError: null,
  activePath: null,
  collapsed: new Set<string>() as ReadonlySet<string>,
  selection: null,
  preview: null,
  submitResult: null,
  stale: false,
  expansions: {},
  composingPath: null,
} as const;

export const useReview = create<ReviewState>((set, get) => {
  const write = async (op: () => Promise<Draft>): Promise<void> => {
    const session = get().sessionId;
    set({ saving: true, saveError: null });
    try {
      const draft = await op();
      if (get().sessionId === session) set({ draft });
    } catch (err) {
      set({ saveError: (err as Error).message });
    } finally {
      set({ saving: false });
    }
  };

  const replaceSession = (updated: SessionSummary) =>
    set((state) => ({
      sessions: (state.sessions ?? [])
        .map((s) => (s.id === updated.id ? updated : s))
        .filter((s) => s.status !== 'closed'),
    }));

  return {
    sessions: null,
    sessionsError: null,
    sessionId: startingSession,
    pickerOpen: false,
    newSessionOpen: false,

    loadSessions: async () => {
      try {
        set({ sessions: await api.listSessions(), sessionsError: null });
      } catch (err) {
        set({ sessionsError: (err as Error).message });
      }
    },

    selectSession: (id) => {
      api.setApiSession(id);
      rememberSessionInUrl(id);
      set({ ...FRESH_VIEW, sessionId: id, pickerOpen: false, status: id ? 'loading' : 'ready' });
      if (id) void get().load();
    },

    setPickerOpen: (pickerOpen) => set({ pickerOpen }),
    setNewSessionOpen: (newSessionOpen) => set({ newSessionOpen, pickerOpen: false }),

    closeSession: async (id) => {
      try {
        replaceSession(await api.closeSession(id));
        if (get().sessionId === id) get().selectSession(null);
      } catch (err) {
        set({ sessionsError: (err as Error).message });
      }
    },

    renameSession: async (id, name) => {
      try {
        replaceSession(await api.updateSession(id, { name }));
      } catch (err) {
        set({ sessionsError: (err as Error).message });
      }
    },

    setSessionBase: async (id, base) => {
      try {
        replaceSession(await api.updateSession(id, { base }));
        if (get().sessionId === id) await get().load();
      } catch (err) {
        set({ saveError: (err as Error).message });
      }
    },

    model: null,
    draft: null,
    status: 'loading',
    error: null,
    saving: false,
    saveError: null,

    activePath: null,
    collapsed: new Set<string>(),
    viewMode: initialViewMode(),
    selection: null,

    preview: null,
    submitting: false,
    submitResult: null,

    sidebarOpen: remembered(SIDEBAR_KEY, true, 'closed'),
    wrap: remembered(WRAP_KEY, true, 'off'),
    shortcutsOpen: false,
    stale: false,
    expansions: {},
    composingPath: null,

    load: async () => {
      const session = get().sessionId;
      if (!session) {
        set({ status: 'ready' });
        return;
      }

      set({ status: 'loading', error: null });
      try {
        const { model, draft } = await api.fetchReview();
        if (get().sessionId !== session) return;
        const previous = get().activePath;

        set({
          model,
          draft,
          status: 'ready',
          stale: false,
          activePath: model.files.some((f) => f.path === previous)
            ? previous
            : (model.files[0]?.path ?? null),
        });
      } catch (err) {
        if (get().sessionId !== session) return;
        set({
          status: err instanceof api.GoneError ? 'gone' : 'error',
          error: (err as Error).message,
        });
      }
    },

    setActivePath: (activePath) => set({ activePath }),

    toggleCollapsed: (path) =>
      set((state) => {
        const next = new Set(state.collapsed);
        if (!next.delete(path)) next.add(path);
        return { collapsed: next };
      }),

    setViewMode: (viewMode) => set({ viewMode, selection: null }),

    beginSelection: (path, side, line) =>
      set({ selection: { path, side, anchorLine: line, headLine: line, dragging: true } }),

    extendSelection: (path, side, line) =>
      set((state) => {
        const current = state.selection;
        if (!current?.dragging || current.path !== path || current.side !== side) return {};
        return { selection: { ...current, headLine: line } };
      }),

    endSelection: () =>
      set((state) =>
        state.selection ? { selection: { ...state.selection, dragging: false } } : {},
      ),

    clearSelection: () => set({ selection: null }),

    setComposingPath: (composingPath) => set({ composingPath }),

    toggleSidebar: () =>
      set((state) => {
        const sidebarOpen = !state.sidebarOpen;
        remember(SIDEBAR_KEY, sidebarOpen, 'open', 'closed');
        return { sidebarOpen };
      }),

    toggleWrap: () =>
      set((state) => {
        const wrap = !state.wrap;
        remember(WRAP_KEY, wrap, 'on', 'off');
        return { wrap };
      }),

    toggleShortcuts: () => set((state) => ({ shortcutsOpen: !state.shortcutsOpen })),

    setStale: (stale) => set({ stale }),

    expandGap: (path, index, edge, size) =>
      set((state) => {
        const key = gapKey(path, index);
        const current = state.expansions[key] ?? { top: 0, bottom: 0 };

        const next: Expansion =
          edge === 'all'
            ? { top: size, bottom: 0 }
            : edge === 'top'
              ? { ...current, top: current.top + EXPAND_STEP }
              : { ...current, bottom: current.bottom + EXPAND_STEP };

        return { expansions: { ...state.expansions, [key]: next } };
      }),

    setViewed: (file, viewed) =>
      write(() => api.putViewed(file.path, viewed ? file.digest : null)),

    openPreview: async () => {
      set({ saveError: null });
      try {
        set({ preview: (await api.fetchPreview()).markdown });
      } catch (err) {
        set({ saveError: (err as Error).message });
      }
    },

    closePreview: () => set({ preview: null }),

    submit: async () => {
      set({ submitting: true, saveError: null });
      try {
        const submitResult = await api.submitReview();
        set({ submitResult, preview: null });
        await Promise.all([get().load(), get().loadSessions()]);
      } catch (err) {
        set({ saveError: (err as Error).message });
      } finally {
        set({ submitting: false });
      }
    },

    dismissResult: () => set({ submitResult: null }),

    addComment: async (input) => {
      await write(() => api.createComment(input));
      set({ selection: null });
    },

    editComment: (id, patch) => write(() => api.updateComment(id, patch)),
    removeComment: (id) => write(() => api.deleteComment(id)),
    setOverall: (text) => write(() => api.putOverall(text)),
  };
});
