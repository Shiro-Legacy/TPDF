import { dropKey, type Key } from './native.js';

export type HeldCert = { key: Key; certs: Buffer[]; cn: string };

/**
 * One decrypted certificate per sender. `take` is one-shot: the caller then owns what it took. A cleared,
 * replaced or idle (five minutes) slot is passed to `drop`, which by default drops (zeroes) its key inside
 * the module; main's hold keeps only a signing-worker handle and drops it there.
 * ponytail: one timer per sender, not a sweep, fine at one window.
 */
export function createCertHold<T = HeldCert>(idleMs = 5 * 60 * 1000, drop: (held: T) => void = held => dropKey((held as HeldCert).key)) {
  const slots = new Map<number, { held: T; timer: ReturnType<typeof setTimeout> }>();
  function remove(id: number): T | undefined {
    const slot = slots.get(id);
    if (!slot) return;
    clearTimeout(slot.timer);
    slots.delete(id);
    return slot.held;
  }
  function clear(id: number): void {
    const held = remove(id);
    if (held !== undefined) drop(held);
  }
  return {
    put(id: number, held: T): void {
      clear(id);
      const timer = setTimeout(() => clear(id), idleMs);
      timer.unref?.();
      slots.set(id, { held, timer });
    },
    /** Removes the slot. A second call is `undefined` (`no-certificate`). */
    take(id: number): T | undefined {
      return remove(id);
    },
    clear,
    clearAll(): void {
      for (const id of [...slots.keys()]) clear(id);
    },
  };
}
