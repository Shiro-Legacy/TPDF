import { dropKey, type Key } from './native.js';

export type HeldCert = { key: Key; certs: Buffer[]; cn: string };

/**
 * One decrypted certificate per sender. `take` is one-shot: the caller then owns the key and drops it after
 * signing. A cleared, replaced or idle (five minutes) slot drops its key, which zeroes it inside the module.
 * ponytail: one timer per sender, not a sweep, fine at one window.
 */
export function createCertHold(idleMs = 5 * 60 * 1000) {
  const slots = new Map<number, { held: HeldCert; timer: ReturnType<typeof setTimeout> }>();
  function remove(id: number): HeldCert | undefined {
    const slot = slots.get(id);
    if (!slot) return;
    clearTimeout(slot.timer);
    slots.delete(id);
    return slot.held;
  }
  function clear(id: number): void {
    dropKey(remove(id)?.key);
  }
  return {
    put(id: number, held: HeldCert): void {
      clear(id);
      const timer = setTimeout(() => clear(id), idleMs);
      timer.unref?.();
      slots.set(id, { held, timer });
    },
    /** Removes the slot. A second call is `undefined` (`no-certificate`). */
    take(id: number): HeldCert | undefined {
      return remove(id);
    },
    clear,
    clearAll(): void {
      for (const id of [...slots.keys()]) clear(id);
    },
  };
}
