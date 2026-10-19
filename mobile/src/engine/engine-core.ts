/**
 * The two helpers the Rust adapter (engine-rust.ts) and the loader (worker-engine.ts) share, as they were in
 * the desktop app's src/renderer/engine-core.ts. That module's other exports (the `/P` bit table, refusal and
 * coercion helpers) served the desktop renderer and the MuPDF test oracle; mobile never used them.
 */

export function attempt<T>(read: () => T, fallback: T): T {
  try {
    return read();
  } catch {
    return fallback;
  }
}

export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
