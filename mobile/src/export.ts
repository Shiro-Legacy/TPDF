import { Capacitor } from '@capacitor/core';
import { Directory, Filesystem } from '@capacitor/filesystem';
import { Share } from '@capacitor/share';

export async function sharePDF(bytes: ArrayBuffer, name: string): Promise<void> {
  const safeName = name.replace(/[\\/:*?"<>|\x00-\x1f]/g, '_').replace(/\.pdf$/i, '').slice(0, 120) + '-edited.pdf';
  if (Capacitor.isNativePlatform()) {
    const data = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(',')[1]);
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(new Blob([bytes], { type: 'application/pdf' }));
    });
    // Cache paths are supported by the Android share provider by default.
    const path = `exports/${Date.now()}/${safeName}`;
    const result = await Filesystem.writeFile({ directory: Directory.Cache, path, data, recursive: true });
    await Share.share({ title: name, files: [result.uri], dialogTitle: 'Share your PDF' });
    // Keep the cache file: a receiving app may read it after the sheet closes.
    return;
  }
  // A download also works when asynchronous PDF serialization loses Web Share's user activation.
  const url = URL.createObjectURL(new Blob([bytes], { type: 'application/pdf' }));
  const anchor = document.createElement('a');
  anchor.href = url; anchor.download = safeName; anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
