/** `redacted`: a redacted copy, unchanged since it was made (Share keeps its name as it is). */
export type LocalDocument = { id: string; name: string; bytes: ArrayBuffer; updated: number; redacted?: boolean };
export type DocumentSummary = Omit<LocalDocument, 'bytes'> & { size: number };
const database = new Promise<IDBDatabase>((resolve, reject) => {
  const request = indexedDB.open('tandem-mobile', 1);
  request.onupgradeneeded = () => request.result.createObjectStore('documents', { keyPath: 'id' });
  request.onsuccess = () => resolve(request.result);
  request.onerror = () => reject(request.error);
});
async function transaction<T>(mode: IDBTransactionMode, action: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await database;
  return new Promise((resolve, reject) => {
    const tx = db.transaction('documents', mode);
    const request = action(tx.objectStore('documents'));
    tx.oncomplete = () => resolve(request.result);
    tx.onabort = tx.onerror = () => reject(tx.error ?? new Error('Could not save on this device. Export your PDF before closing.'));
  });
}
export async function listDocuments(): Promise<DocumentSummary[]> {
  const db = await database;
  return new Promise((resolve, reject) => {
    const tx = db.transaction('documents', 'readonly');
    const cursor = tx.objectStore('documents').openCursor();
    const summaries: DocumentSummary[] = [];
    cursor.onsuccess = () => {
      const item = cursor.result;
      if (!item) return;
      const doc = item.value as LocalDocument;
      summaries.push({ id: doc.id, name: doc.name, updated: doc.updated, size: doc.bytes.byteLength });
      item.continue();
    };
    tx.oncomplete = () => resolve(summaries);
    tx.onabort = tx.onerror = () => reject(tx.error);
  });
}
export const getDocument = (id: string) => transaction<LocalDocument | undefined>('readonly', store => store.get(id));
export const putDocument = (doc: LocalDocument) => transaction('readwrite', store => store.put(doc));
export async function putDocuments(docs: LocalDocument[]): Promise<void> {
  const db = await database;
  await new Promise<void>((resolve, reject) => {
    const unavailable = () => reject(new Error('Device storage is full or unavailable. Your scans are still here; free some space and retry.'));
    let tx: IDBTransaction;
    try { tx = db.transaction('documents', 'readwrite'); } catch { unavailable(); return; }
    tx.oncomplete = () => resolve();
    tx.onabort = tx.onerror = unavailable;
    try { for (const doc of docs) tx.objectStore('documents').put(doc); }
    catch { tx.abort(); unavailable(); }
  });
}
export const removeDocument = (id: string) => transaction('readwrite', store => store.delete(id));
