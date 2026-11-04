export type FileStatus =
  | 'added'
  | 'modified'
  | 'deleted'
  | 'renamed'
  | 'untracked'
  | 'unmerged';

export type SkipReason = 'binary' | 'too-large' | 'unmerged' | 'submodule' | null;

export type LineType = 'context' | 'add' | 'delete';

export interface DiffLine {
  type: LineType;
  oldLineNo: number | null;
  newLineNo: number | null;
  content: string;
  noNewlineAtEof?: boolean;
}

export interface Hunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  oldPath: string | null;
  status: FileStatus;
  similarity: number | null;
  language: string | null;
  binary: boolean;
  skipped: SkipReason;
  additions: number;
  deletions: number;
  oldLineCount: number;
  newLineCount: number;
  oldSize: number;
  newSize: number;
  digest: string;
  hunks: Hunk[];
}

export interface ReviewBase {
  rev: string;
  sha: string | null;
}

export interface ReviewModel {
  version: 1;
  repoRoot: string;
  base: ReviewBase;
  generatedAt: string;
  includeUntracked: boolean;
  contextLines: number;
  files: FileDiff[];
  totals: {
    files: number;
    additions: number;
    deletions: number;
    skipped: number;
  };
}

export type Side = 'old' | 'new';

export type CommentTag = 'must-fix' | 'question' | 'nit' | 'idea';

export const COMMENT_TAGS: readonly CommentTag[] = ['must-fix', 'question', 'nit', 'idea'];

export interface CommentAnchor {
  side: Side;
  startLine: number;
  endLine: number;
  lineHashes: string[];

  lineTexts: string[];
  beforeHashes: string[];
  afterHashes: string[];
}

export type AnchorState = 'anchored' | 'moved' | 'orphaned';

export interface Comment {
  id: string;
  path: string;
  anchor: CommentAnchor | null;
  tag: CommentTag;
  body: string;
  createdAt: string;
  updatedAt: string;
  state?: AnchorState;
  resolvedStartLine?: number;
  resolvedEndLine?: number;
}

export interface Draft {
  version: 1;
  overall: string;
  comments: Comment[];
  viewed: Record<string, string>;
  updatedAt: string;
}

export interface ReviewPayload {
  model: ReviewModel;
  draft: Draft;
}

export type SessionStatus = 'open' | 'submitted' | 'closed';

export interface SessionSummary {
  id: string;
  name: string;
  path: string;
  commonDir: string;
  /** `auto`, or a revision. */
  base: string;
  includeUntracked: boolean;
  createdBy: 'user' | 'claude';
  status: SessionStatus;
  createdAt: string;
  updatedAt: string;
  submittedAt: string | null;
  takenAt: string | null;
  rounds: number;
  branch: string | null;
  missing: boolean;
  draftComments: number;
}

export interface Suggestion {
  path: string;
  source: 'claude' | 'worktree';
  branch: string | null;
}

export interface CreatedSession {
  session: SessionSummary;
  reused: boolean;
  notes: string[];
}
