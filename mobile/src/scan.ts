import { Capacitor } from '@capacitor/core';
import { DocumentScanner } from './scan-native.js';
import { MAX_IMAGE_BYTES, MAX_SCAN_BYTES, MAX_SCAN_PAGES, MAX_PAGE_TEXT, parseDocument, validateLines, validatePages, type ScanPage, type ScanOutput } from './scan-model.js';
import { AI_PROVIDERS, aiProvider, reconstruct, providerText, TRANSCRIPTION_PROMPT, type AISettings } from './scan-ai.js';

import { ChatGPT, chatGPTMode, setupChatGPT, type ChatGPTSettings } from './chatgpt.js';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const native = Capacitor.isNativePlatform();
let pages: ScanPage[] = [], selected = 0, working = false, stopRequested = false;
let settings: AISettings | ChatGPTSettings | null = null;
const dialog = () => $<HTMLDialogElement>('scan-dialog');
const message = (text: string) => { $('scan-status').textContent = text; };
const output = () => $<HTMLSelectElement>('scan-output').value as ScanOutput;
function refreshAIFields() {
  const provider = aiProvider($<HTMLSelectElement>('ai-provider').value as AISettings['provider']);
  $('ai-model-hint').textContent = provider.modelHint;
  $('ai-billing-note').textContent = provider.billingNote;
}
function refresh() {
  const current = pages[selected];
  $('scan-empty').hidden = !!current; $('scan-review').hidden = !current;
  $('scan-page-label').textContent = `${selected + 1} / ${pages.length}`;
  if (current) $<HTMLImageElement>('scan-image').src = current.data;
  $<HTMLButtonElement>('scan-prev').disabled = selected === 0;
  $<HTMLButtonElement>('scan-next').disabled = selected >= pages.length - 1;
  $<HTMLButtonElement>('scan-earlier').disabled = selected === 0;
  $<HTMLButtonElement>('scan-later').disabled = selected >= pages.length - 1;
  $('scan-ai-options').hidden = output() !== 'digital';
  $('scan-ocr-note').hidden = output() !== 'searchable';
  $('scan-text-review').hidden = !current || output() === 'image' || (output() === 'searchable' ? !current.lines : current.markdown === undefined);
  $('scan-text-label').textContent = output() === 'digital' ? 'Review and edit the rebuilt page' : 'Review and correct recognized lines';
  $('scan-text-help').textContent = output() === 'digital' ? 'Use # for headings and | for tables. Review names, numbers, and [unclear] text. Figures remain in the original scan.' : 'Each line stays aligned to its place in the scan. Correct spelling without adding or removing lines.';
  const editor = $('scan-text-fields'); editor.replaceChildren();
  if (current && output() === 'digital' && current.markdown !== undefined) {
    const area = document.createElement('textarea'); area.rows = 12; area.maxLength = MAX_PAGE_TEXT; area.value = current.markdown; area.setAttribute('aria-label', 'Rebuilt page text');
    area.oninput = () => { current.markdown = area.value; updatePreview(); };
    editor.append(area); updatePreview();
  } else if (current?.lines && output() === 'searchable') {
    current.lines.forEach((line, index) => {
      const area = document.createElement('textarea'); area.rows = 1; area.maxLength = 4000; area.value = line.text; area.setAttribute('aria-label', `Recognized line ${index + 1}`);
      const resize = () => { area.style.height = 'auto'; area.style.height = `${Math.max(48, Math.min(180, area.scrollHeight + 2))}px`; };
      area.oninput = () => { line.text = area.value; resize(); };
      editor.append(area);
      requestAnimationFrame(resize);
    });
  }
  $('scan-layout-preview').hidden = output() !== 'digital';
  const ready = pages.length && (output() === 'image' || pages.every(p => output() === 'searchable' ? p.lines !== undefined : !!p.markdown?.trim()));
  $<HTMLButtonElement>('scan-save').disabled = !ready;
  $('scan-process').hidden = output() === 'image';
  $<HTMLButtonElement>('scan-process').disabled = !pages.length;
  $('scan-process').textContent = output() === 'digital' ? 'Send to AI & rebuild' : 'Recognize text on device';
  $('scan-review-check-label').hidden = output() === 'image';
  $('scan-original-note').hidden = output() !== 'digital';
  const provider = settings ? settings.provider === 'chatgpt' ? { label: 'ChatGPT' } : aiProvider(settings.provider) : null;
  $('scan-ai-state').textContent = settings ? settings.provider === 'chatgpt' ? `Using ChatGPT plan · ${settings.accountLabel} · ${settings.model}` : `${provider!.label} · ${settings.model} · connected for this session` : 'Connect your ChatGPT plan or choose a provider with your own API key.';
  $('scan-manage-usage').hidden = settings?.provider !== 'chatgpt';
  $('scan-ai-connect').textContent = settings ? 'Change AI connection' : 'Connect AI provider';
  $('scan-consent-text').textContent = provider ? `Send these page images to ${provider.label}. Provider API charges or eligible credits and the provider’s data policies apply.` : 'Connect an AI provider, then allow these page images to be sent to it. Provider API charges or eligible credits and data policies apply.';
  if (settings?.provider === 'chatgpt') $('scan-consent-text').textContent = 'Send these page images to OpenAI using my ChatGPT plan allowance or available credits. OpenAI’s data policies apply.';
  if (settings?.provider === 'meta' && /contributor/i.test(settings.model)) $('scan-consent-text').textContent += ' This Contributor model also allows Meta to use these images and results to train its models.';
  $<HTMLInputElement>('scan-consent').disabled = !settings;
}
function updatePreview() {
  const preview = $('scan-layout-preview'); preview.replaceChildren();
  try {
    for (const block of parseDocument(pages[selected]?.markdown || '')) {
      if (block.kind === 'table') {
        const table = document.createElement('table');
        block.rows.forEach((row, index) => { const tr = document.createElement('tr'); row.forEach(value => { const cell = document.createElement(index ? 'td' : 'th'); cell.textContent = value; tr.append(cell); }); table.append(tr); });
        preview.append(table);
      } else {
        const element = document.createElement(block.kind === 'heading' ? `h${block.level}` : 'p'); element.textContent = block.text; preview.append(element);
      }
    }
  } catch (error) { preview.textContent = (error as Error).message; }
}
async function task(action: () => Promise<void>) {
  if (working) return;
  working = true; stopRequested = false;
  $('scan-content').inert = true; $('scan-close').hidden = true; $('scan-stop').hidden = false;
  dialog().setAttribute('aria-busy', 'true');
  try { await action(); }
  catch (error) { message(error instanceof Error ? error.message : 'That operation failed. Your scans are still here.'); }
  finally {
    working = false; $('scan-content').inert = false; $('scan-close').hidden = false; $('scan-stop').hidden = true;
    dialog().setAttribute('aria-busy', 'false'); refresh();
  }
}
async function loadImage(source: string) {
  const image = new Image(); image.src = source;
  try { await image.decode(); } catch { throw new Error('This image could not be read. Try a JPEG or PNG.'); }
  if (!image.naturalWidth || !image.naturalHeight || image.naturalWidth * image.naturalHeight > 100_000_000) throw new Error('This photo is too large. Choose an image under 100 megapixels.');
  return image;
}
async function normalized(source: string, rotate = false, crop?: { x: number; y: number; width: number; height: number }): Promise<Pick<ScanPage, 'data' | 'width' | 'height'>> {
  const image = await loadImage(source);
  const rect = crop || { x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight };
  const scale = Math.min(1, 2400 / Math.max(rect.width, rect.height));
  const width = Math.max(1, Math.round(rect.width * scale)), height = Math.max(1, Math.round(rect.height * scale));
  const canvas = document.createElement('canvas'); canvas.width = rotate ? height : width; canvas.height = rotate ? width : height;
  const context = canvas.getContext('2d')!; context.fillStyle = 'white'; context.fillRect(0, 0, canvas.width, canvas.height);
  if (rotate) { context.translate(canvas.width, 0); context.rotate(Math.PI / 2); }
  context.drawImage(image, rect.x, rect.y, rect.width, rect.height, 0, 0, width, height);
  const result = { data: canvas.toDataURL('image/jpeg', .88), width: canvas.width, height: canvas.height };
  canvas.width = canvas.height = 1;
  return result;
}
async function addImages(sources: string[]) {
  if (pages.length + sources.length > MAX_SCAN_PAGES) throw new Error(`A scan can contain up to ${MAX_SCAN_PAGES} pages. Remove a page first.`);
  const additions: ScanPage[] = [];
  for (const source of sources) additions.push({ id: crypto.randomUUID(), ...await normalized(source) });
  if ([...pages, ...additions].reduce((sum, p) => sum + p.data.length * .75, 0) > MAX_SCAN_BYTES) throw new Error('These scans exceed 30 MB. Choose fewer images.');
  pages.push(...additions); selected = pages.length - additions.length; invalidateReview(); message(`${pages.length} page${pages.length === 1 ? '' : 's'} ready. Choose how to save them.`);
  $<HTMLInputElement>('scan-consent').checked = false;
}
function invalidateReview() { $<HTMLInputElement>('scan-reviewed').checked = false; }
function pdf(pages: ScanPage[], output: ScanOutput): Promise<ArrayBuffer> {
  validatePages(pages, output);
  return new Promise((resolve, reject) => {
    const worker = new Worker('scan-worker.js', { type: 'module' });
    const timer = setTimeout(() => { worker.terminate(); reject(new Error('PDF creation timed out. Try fewer pages.')); }, 90_000);
    const finish = () => { clearTimeout(timer); worker.terminate(); };
    worker.onerror = () => { finish(); reject(new Error('PDF creation failed. Your scans are still here.')); };
    worker.onmessage = ({ data }) => {
      if (data.ready) { worker.postMessage({ pages, output }); return; }
      finish(); if (data.error) reject(new Error(data.error)); else resolve(data.bytes);
    };
  });
}

export function setupScanner(save: (bytes: ArrayBuffer, name: string, original?: ArrayBuffer) => Promise<void>) {
  const providerSelect = $<HTMLSelectElement>('ai-provider');
  providerSelect.replaceChildren(...AI_PROVIDERS.map(provider => {
    const option = document.createElement('option'); option.value = provider.id; option.textContent = provider.label; return option;
  }));
  const resetConnection = () => { settings = null; $<HTMLInputElement>('scan-consent').checked = false; refresh(); };
  const chatgpt = setupChatGPT(resetConnection);
  refreshAIFields(); chatgpt.controls();
  window.addEventListener('beforeunload', event => { if (pages.length) { event.preventDefault(); event.returnValue = ''; } });
  $('scan').onclick = () => { refresh(); dialog().showModal(); };
  $('scan-close').onclick = () => { if (!working) dialog().close(); };
  dialog().addEventListener('cancel', event => { if (working) { event.preventDefault(); stopRequested = true; message('Stopping after the current page…'); } });
  dialog().addEventListener('close', () => { if (working) { stopRequested = true; message('Stopping after the current page…'); dialog().showModal(); } });
  $('scan-stop').onclick = () => { stopRequested = true; message('Stopping after the current page. A page already sent to AI may still be billed.'); };
  $('scan-discard').onclick = () => { if (confirm('Discard these unsaved scans?')) { pages = []; selected = 0; invalidateReview(); message(''); refresh(); } };
  $('scan-output').onchange = () => { invalidateReview(); refresh(); };
  $('scan-ocr-note').textContent = native ? (Capacitor.getPlatform() === 'android' ? 'Offline OCR recognizes Latin-script text (including English). Review it before saving.' : 'Text recognition runs on your iPhone. Supported languages depend on iOS. Review it before saving.') : 'On-device OCR is available in the installed phone app. This browser preview can create image PDFs.';
  $('scan-camera').onclick = () => {
    if (!native) { $<HTMLInputElement>('scan-camera-input').click(); return; }
    void task(async () => { message('Opening the document scanner…'); const result = await DocumentScanner.scan({ pageLimit: MAX_SCAN_PAGES - pages.length }); if (result.images.length) await addImages(result.images.map(data => `data:image/jpeg;base64,${data}`)); else message('Scanning cancelled.'); });
  };
  $('scan-import').onclick = () => $<HTMLInputElement>('scan-images-input').click();
  for (const id of ['scan-images-input', 'scan-camera-input']) {
    const input = $<HTMLInputElement>(id);
    input.onchange = () => {
      const files = Array.from(input.files || []); input.value = '';
      if (!files.length) return;
      void task(async () => {
        if (files.length + pages.length > MAX_SCAN_PAGES) throw new Error(`Choose up to ${MAX_SCAN_PAGES - pages.length} more images.`);
        if (files.some(file => file.size > MAX_IMAGE_BYTES)) throw new Error('Choose photos smaller than 20 MB each.');
        const urls = files.map(file => URL.createObjectURL(file));
        try { await addImages(urls); } finally { urls.forEach(URL.revokeObjectURL); }
      });
    };
  }
  $('scan-sample').onclick = () => void task(async () => { const response = await fetch('sample-scan.jpg'); if (!response.ok) throw new Error('Sample scan unavailable.'); const url = URL.createObjectURL(await response.blob()); try { await addImages([url]); } finally { URL.revokeObjectURL(url); } });
  for (const [id, delta] of [['scan-prev', -1], ['scan-next', 1]] as const) $(id).onclick = () => { selected = Math.max(0, Math.min(pages.length - 1, selected + delta)); refresh(); };
  for (const [id, delta] of [['scan-earlier', -1], ['scan-later', 1]] as const) $(id).onclick = () => { const next = selected + delta; if (next < 0 || next >= pages.length) return; [pages[selected], pages[next]] = [pages[next], pages[selected]]; selected = next; invalidateReview(); refresh(); };
  $('scan-remove').onclick = () => { pages.splice(selected, 1); selected = Math.max(0, Math.min(selected, pages.length - 1)); invalidateReview(); refresh(); };
  $('scan-rotate').onclick = () => void task(async () => { const current = pages[selected]; Object.assign(current, await normalized(current.data, true)); delete current.lines; delete current.markdown; invalidateReview(); message('Page rotated. Recognize this page again if needed.'); });
  setupCrop();
  $('scan-ai-connect').onclick = () => {
    $<HTMLInputElement>('ai-key').value = '';
    if (settings) {
      $<HTMLSelectElement>('ai-provider').value = settings.provider === 'chatgpt' ? 'openai' : settings.provider;
      $<HTMLSelectElement>('ai-openai-method').value = settings.provider === 'chatgpt' ? 'chatgpt' : 'api';
      if (settings.provider !== 'chatgpt') $<HTMLInputElement>('ai-model').value = settings.model;
    }
    refreshAIFields(); chatgpt.controls();
    $<HTMLDialogElement>('ai-dialog').showModal();
    void chatgpt.open(settings?.provider === 'chatgpt' ? settings : undefined);
  };
  const changeConnection = () => {
    chatgpt.cancel(); resetConnection();
    $<HTMLInputElement>('ai-model').value = '';
    $<HTMLInputElement>('ai-key').value = '';
    refreshAIFields(); chatgpt.controls(); void chatgpt.open();
  };
  $('ai-provider').onchange = changeConnection;
  $('ai-openai-method').onchange = changeConnection;
  $('ai-form').onsubmit = event => {
    event.preventDefault();
    if (chatGPTMode()) {
      const connection = chatgpt.candidate(); if (!connection) return;
      settings = connection;
    } else {
      settings = { provider: $<HTMLSelectElement>('ai-provider').value as AISettings['provider'], model: $<HTMLInputElement>('ai-model').value.trim(), key: $<HTMLInputElement>('ai-key').value.trim() };
    }
    $<HTMLInputElement>('scan-consent').checked = false;
    $<HTMLInputElement>('ai-key').value = ''; $<HTMLDialogElement>('ai-dialog').close(); refresh();
  };
  $<HTMLDialogElement>('ai-dialog').addEventListener('close', () => { $<HTMLInputElement>('ai-key').value = ''; chatgpt.cancel(); });
  $('ai-disconnect').onclick = () => { resetConnection(); $<HTMLInputElement>('ai-key').value = ''; $<HTMLInputElement>('ai-model').value = ''; $<HTMLDialogElement>('ai-dialog').close(); };
  $('scan-reprocess').onclick = () => {
    const current = pages[selected];
    if (!current || !confirm('Replace the recognized text for this page? Any corrections on this page will be lost.')) return;
    if (output() === 'digital') delete current.markdown; else delete current.lines;
    invalidateReview(); refresh(); message('Use the process button to recognize this page again.');
  };
  $('scan-process').onclick = () => void task(async () => {
    const type = output();
    if (type === 'digital' && !settings) throw new Error('Connect your ChatGPT plan or an AI provider API key first.');
    if (type === 'digital' && !$<HTMLInputElement>('scan-consent').checked) throw new Error('Confirm that these images may be sent to your selected AI provider.');
    if (type === 'searchable' && !native) throw new Error('On-device OCR requires the installed iPhone or Android app.');
    invalidateReview();
    for (let i = 0; i < pages.length; i++) {
      if (stopRequested) break;
      if (type === 'digital' ? pages[i].markdown !== undefined : pages[i].lines !== undefined) continue;
      message(`${type === 'digital' ? 'Rebuilding' : 'Recognizing'} page ${i + 1} of ${pages.length}…`);
      if (type === 'digital') {
        const connection = settings!;
        if (connection.provider === 'chatgpt') {
          if (!native) throw new Error('ChatGPT plan requests require the installed phone app.');
          const result = await ChatGPT.reconstruct({ accountId: connection.accountId, model: connection.model, image: pages[i].data, instructions: TRANSCRIPTION_PROMPT });
          pages[i].markdown = providerText('openai', { status: 200, data: result.response, headers: {}, url: '' });
        } else pages[i].markdown = await reconstruct(connection, pages[i].data);
      }
      else pages[i].lines = validateLines((await DocumentScanner.recognize({ data: pages[i].data.split(',')[1] })).lines);
      selected = i;
    }
    const emptyPages = pages.filter(p => p.lines?.length === 0).length;
    message(stopRequested ? 'Stopped. Completed pages are preserved. Tap the process button to continue.' : emptyPages && type === 'searchable' ? `${emptyPages} page(s) had no recognizable text. Their images will be preserved. Review each page before saving.` : 'Ready to review. Check every page, especially names, numbers, and unclear text.');
  });
  $('scan-save').onclick = () => void task(async () => {
    const type = output();
    if (type !== 'image' && !$<HTMLInputElement>('scan-reviewed').checked) throw new Error('Review every page, then confirm below before saving.');
    message('Creating your PDF…');
    const bytes = await pdf(pages, type);
    const original = type === 'digital' ? await pdf(pages, 'image') : undefined;
    const name = ($<HTMLInputElement>('scan-name').value.trim() || 'Scanned document').replace(/\.pdf$/i, '').slice(0, 100);
    await save(bytes, `${name}${type === 'digital' ? ' — rebuilt' : ''}.pdf`, original);
    pages = []; selected = 0; invalidateReview(); $<HTMLInputElement>('scan-consent').checked = false;
    working = false; dialog().close(); message('');
  });
}

function setupCrop() {
  const cropDialog = $<HTMLDialogElement>('crop-dialog'), canvas = $<HTMLCanvasElement>('crop-canvas');
  let image: HTMLImageElement, start: [number, number] | null = null, end: [number, number] | null = null;
  const point = (e: PointerEvent): [number, number] => { const r = canvas.getBoundingClientRect(); return [Math.max(0, Math.min(canvas.width, (e.clientX - r.left) * canvas.width / r.width)), Math.max(0, Math.min(canvas.height, (e.clientY - r.top) * canvas.height / r.height))]; };
  function paint() {
    const ctx = canvas.getContext('2d')!; ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
    if (start && end) { ctx.strokeStyle = '#168952'; ctx.lineWidth = 4; ctx.strokeRect(start[0], start[1], end[0] - start[0], end[1] - start[1]); }
  }
  $('scan-crop').onclick = () => void task(async () => {
    image = await loadImage(pages[selected].data); canvas.width = image.naturalWidth / Math.max(image.naturalWidth, image.naturalHeight) * 900; canvas.height = image.naturalHeight / Math.max(image.naturalWidth, image.naturalHeight) * 900;
    start = end = null; paint(); cropDialog.showModal();
  });
  canvas.onpointerdown = event => { canvas.setPointerCapture(event.pointerId); start = end = point(event); paint(); };
  canvas.onpointermove = event => { if (canvas.hasPointerCapture(event.pointerId)) { end = point(event); paint(); } };
  $('crop-apply').onclick = () => void task(async () => {
    if (!start || !end || Math.abs(end[0] - start[0]) < 25 || Math.abs(end[1] - start[1]) < 25) { $('crop-help').textContent = 'Drag a rectangle around the page first.'; return; }
    const sx = image.naturalWidth / canvas.width, sy = image.naturalHeight / canvas.height;
    const crop = { x: Math.min(start[0], end[0]) * sx, y: Math.min(start[1], end[1]) * sy, width: Math.abs(end[0] - start[0]) * sx, height: Math.abs(end[1] - start[1]) * sy };
    Object.assign(pages[selected], await normalized(pages[selected].data, false, crop)); delete pages[selected].lines; delete pages[selected].markdown;
    invalidateReview(); cropDialog.close(); message('Page cropped. Recognize this page again if needed.');
  });
}
