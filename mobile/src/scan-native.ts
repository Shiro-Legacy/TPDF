import { registerPlugin } from '@capacitor/core';
import type { OCRLine } from './scan-model.js';
export const DocumentScanner = registerPlugin<{
  scan(options: { pageLimit: number }): Promise<{ images: string[] }>;
  recognize(options: { data: string }): Promise<{ lines: OCRLine[] }>;
}>('DocumentScanner');
