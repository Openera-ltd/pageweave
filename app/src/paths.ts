// Minimal path helpers that work for both Windows ("C:\\a\\b.pdf") and POSIX paths.

export function sepOf(p: string): string {
  return p.includes("\\") && !p.includes("/") ? "\\" : p.includes("\\") ? "\\" : "/";
}

export function basename(p: string): string {
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1] ?? p;
}

export function dirname(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  if (i < 0) return "";
  if (i === 0) return p.slice(0, 1);
  // Keep "C:\" for drive roots.
  if (i === 2 && p[1] === ":") return p.slice(0, 3);
  return p.slice(0, i);
}

export function stem(p: string): string {
  const b = basename(p);
  const dot = b.lastIndexOf(".");
  return dot > 0 ? b.slice(0, dot) : b;
}

export function join(dir: string, name: string): string {
  if (!dir) return name;
  const sep = sepOf(dir);
  return dir.endsWith("/") || dir.endsWith("\\") ? dir + name : dir + sep + name;
}

/** Default output path next to the source: "report.pdf" → "report-extract.pdf". */
export function siblingOutput(source: string, suffix: string): string {
  return join(dirname(source), `${stem(source)}-${suffix}.pdf`);
}
