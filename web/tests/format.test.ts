import { describe, expect, it } from 'vitest';
import { describeBase, timeAgo, tidyPath } from '../src/format.ts';

describe('timeAgo', () => {
  const now = Date.parse('2026-01-01T12:00:00Z');

  it('rounds to the nearest sensible unit', () => {
    expect(timeAgo('2026-01-01T11:59:50Z', now)).toBe('just now');
    expect(timeAgo('2026-01-01T11:55:00Z', now)).toBe('5m ago');
    expect(timeAgo('2026-01-01T09:00:00Z', now)).toBe('3h ago');
    expect(timeAgo('2025-12-30T12:00:00Z', now)).toBe('2d ago');
  });

  it('is empty for a missing or unparseable time', () => {
    expect(timeAgo(null, now)).toBe('');
    expect(timeAgo('not a date', now)).toBe('');
  });
});

describe('tidyPath', () => {
  it('shortens home directories', () => {
    expect(tidyPath('/Users/sam/code/repo')).toBe('~/code/repo');
    expect(tidyPath('/home/sam/repo')).toBe('~/repo');
  });

  it('leaves other paths alone', () => {
    expect(tidyPath('/srv/repo')).toBe('/srv/repo');
  });
});

describe('describeBase', () => {
  it('names the two built-in bases', () => {
    expect(describeBase('auto')).toBe('branch changes');
    expect(describeBase('HEAD')).toBe('uncommitted changes');
    expect(describeBase('origin/main')).toBe('origin/main');
  });
});
