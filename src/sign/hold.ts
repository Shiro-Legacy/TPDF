import type { KeyObject } from 'node:crypto';

export type HeldCert = { key: KeyObject; certs: Buffer[]; cn: string };

/**
 * One decrypted certificate per sender. `take` is one-shot. Idle drop is a
 * process timer — ponytail: one timer per sender, not a sweep, fine at one window.
 */
export function createCertHold(idleMs = 5 * 60 * 1000) {
  const slots = new Map<number, { held: HeldCert; timer: ReturnType<typeof setTimeout> }>();
  function clear(id: number): void {
    const slot = slots.get(id);
    if (!slot) return;
    clearTimeout(slot.timer);
    slots.delete(id);
  }
  return {
    put(id: number, held: HeldCert): void {
      clear(id);
      const timer = setTimeout(() => { slots.delete(id); }, idleMs);
      timer.unref?.();
      slots.set(id, { held, timer });
    },
    /** Removes the slot. A second call is `undefined` (`no-certificate`). */
    take(id: number): HeldCert | undefined {
      const slot = slots.get(id);
      if (!slot) return;
      clear(id);
      return slot.held;
    },
    clear,
    clearAll(): void {
      for (const id of [...slots.keys()]) clear(id);
    },
  };
}
