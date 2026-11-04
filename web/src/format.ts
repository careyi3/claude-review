import type { AnchorState, CommentTag, FileDiff, SkipReason } from './types.ts';

export const STATUS_LABEL: Record<FileDiff['status'], string> = {
  added: 'added',
  modified: 'modified',
  deleted: 'deleted',
  renamed: 'renamed',
  untracked: 'new',
  unmerged: 'conflict',
};

export const STATUS_COLOR: Record<FileDiff['status'], string> = {
  added: 'text-emerald-600 dark:text-emerald-400',
  modified: 'text-amber-600 dark:text-amber-400',
  deleted: 'text-rose-600 dark:text-rose-400',
  renamed: 'text-violet-600 dark:text-violet-400',
  untracked: 'text-emerald-600 dark:text-emerald-400',
  unmerged: 'text-rose-600 dark:text-rose-400',
};

const SKIP_TEXT: Record<Exclude<SkipReason, null>, string> = {
  binary: 'Binary file — not shown',
  'too-large': 'File is larger than the size limit — not shown',
  submodule: 'Submodule — not shown',
  unmerged: 'Unmerged conflict — resolve it before reviewing',
};

export function skipMessage(reason: Exclude<SkipReason, null>): string {
  return SKIP_TEXT[reason];
}

export function displayPath(file: FileDiff): string {
  return file.oldPath ? `${file.oldPath} → ${file.path}` : file.path;
}

export function splitPath(path: string): { dir: string; name: string } {
  const idx = path.lastIndexOf('/');
  return idx === -1
    ? { dir: '', name: path }
    : { dir: path.slice(0, idx + 1), name: path.slice(idx + 1) };
}

export const TAG_STYLE: Record<CommentTag, React.CSSProperties> = {
  'must-fix': { background: '#b4231f', color: '#fff' },
  question: { background: '#1f6feb', color: '#fff' },
  nit: { background: '#8250df', color: '#fff' },
  idea: { background: '#0f7b6c', color: '#fff' },
};

export const STATE_NOTE: Record<AnchorState, string | null> = {
  anchored: null,
  moved: 'the file changed; this comment followed its line',
  orphaned: 'the line this was written against is gone',
};

export function timeAgo(iso: string | null, now = Date.now()): string {
  if (!iso) return '';
  const seconds = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (Number.isNaN(seconds)) return '';
  if (seconds < 45) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

/** Shortens a home-directory path to `~/…` when the home directory is known. */
export function tidyPath(path: string): string {
  const home = /^(\/Users\/[^/]+|\/home\/[^/]+)/.exec(path)?.[1];
  return home ? `~${path.slice(home.length)}` : path;
}

export function describeBase(base: string): string {
  if (base === 'auto') return 'branch changes';
  if (base === 'HEAD') return 'uncommitted changes';
  return base;
}
