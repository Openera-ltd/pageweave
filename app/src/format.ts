// Small display formatters shared by panels, dialogs and notices.

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** "1.2 MB → 640.0 KB (47% smaller)"; the percentage is rounded down so it never overstates. */
export function sizeChange(before: number, after: number): string {
  const pct = before > 0 ? Math.floor((100 * (before - after)) / before) : 0;
  return `${formatBytes(before)} → ${formatBytes(after)} (${pct > 0 ? `${pct}% smaller` : "less than 1% smaller"})`;
}
