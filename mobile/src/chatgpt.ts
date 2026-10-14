import { Capacitor, registerPlugin } from '@capacitor/core';

export type ChatGPTAccountInfo = { id: string; email: string; label: string; connected: boolean; planEnabled: boolean };
export type ChatGPTAccounts = { accounts: ChatGPTAccountInfo[]; activeAccountId?: string };
export type ChatGPTSettings = { provider: 'chatgpt'; accountId: string; accountLabel: string; model: string };
interface ChatGPTBridge {
  getAccounts(): Promise<ChatGPTAccounts>;
  signIn(options: { accountId?: string }): Promise<ChatGPTAccounts>;
  cancelSignIn(): Promise<void>;
  selectAccount(options: { accountId: string }): Promise<ChatGPTAccounts>;
  signOut(options: { accountId: string }): Promise<ChatGPTAccounts & { revoked: boolean }>;
  models(options: { accountId: string }): Promise<{ models: { id: string; name: string }[] }>;
  reconstruct(options: { accountId: string; model: string; image: string; instructions: string }): Promise<{ response: unknown }>;
  manageUsage(): Promise<void>;
}
export const ChatGPT = registerPlugin<ChatGPTBridge>('ChatGPTAccount');
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const native = Capacitor.isNativePlatform();
export function chatGPTMode() { return $<HTMLSelectElement>('ai-provider').value === 'openai' && $<HTMLSelectElement>('ai-openai-method').value === 'chatgpt'; }
export function chatGPTError(error: unknown) {
  return error && typeof error === 'object' && 'message' in error && typeof error.message === 'string' ? error.message : 'ChatGPT could not complete this request. Your scans are still here.';
}
export async function manageChatGPTUsage() {
  if (!native) throw new Error('Manage your ChatGPT plan in the installed phone app.');
  await ChatGPT.manageUsage();
}

// Account metadata and model names only. OAuth credentials never cross this bridge.
export function setupChatGPT(onConnectionChanged: () => void) {
  let snapshot: ChatGPTAccounts = { accounts: [] };
  let modelList: { id: string; name: string }[] = [];
  let busy = false, authorizing = false, generation = 0;
  const status = (text: string) => { $('chatgpt-status').textContent = text; };
  const account = () => snapshot.accounts.find(item => item.id === $<HTMLSelectElement>('chatgpt-accounts').value);
  function controls() {
    const current = account(), plan = chatGPTMode();
    $('ai-method-field').hidden = $<HTMLSelectElement>('ai-provider').value !== 'openai';
    $('chatgpt-panel').hidden = !plan;
    $('ai-api-fields').hidden = plan;
    for (const id of ['ai-model', 'ai-key']) $<HTMLInputElement>(id).required = !plan;
    $('ai-account-note').textContent = plan
      ? 'Use an eligible ChatGPT Plus or Pro plan. Sign in securely with OpenAI and choose whether Tandem may use your plan. Your existing chats are not shared with Tandem.'
      : 'Use your own API account. API charges or eligible provider credits apply. Your key is held only for this app session and sent directly to the provider.';
    $<HTMLSelectElement>('chatgpt-accounts').disabled = busy || !snapshot.accounts.length;
    $<HTMLSelectElement>('chatgpt-model').disabled = busy || !current?.planEnabled || !current.connected;
    $<HTMLButtonElement>('chatgpt-signin').disabled = busy || !native;
    $<HTMLButtonElement>('chatgpt-reconnect').disabled = busy || !current || !native;
    $('chatgpt-reconnect').textContent = current?.connected && !current.planEnabled ? 'Enable ChatGPT plan usage' : 'Sign in again';
    $<HTMLButtonElement>('chatgpt-signout').disabled = busy || !current?.connected;
    $<HTMLButtonElement>('chatgpt-refresh-models').disabled = busy || !current?.connected || !current.planEnabled;
    $('chatgpt-cancel').hidden = !authorizing;
    $<HTMLButtonElement>('ai-submit').disabled = plan && (busy || !current?.connected || !current.planEnabled || !modelList.some(m => m.id === $<HTMLSelectElement>('chatgpt-model').value));
    $('ai-submit').textContent = plan ? 'Use ChatGPT plan' : 'Use API key for this session';
  }
  function render(next: ChatGPTAccounts, preferred?: string) {
    snapshot = next;
    const select = $<HTMLSelectElement>('chatgpt-accounts');
    select.replaceChildren(...next.accounts.map(item => {
      const option = document.createElement('option'); option.value = item.id;
      option.textContent = `${item.email || item.label || 'ChatGPT account'} · ${item.label || item.id.slice(-8)}${item.connected ? '' : ' · signed out'}`;
      return option;
    }));
    const selected = preferred || next.activeAccountId;
    if (selected && next.accounts.some(item => item.id === selected)) select.value = selected;
    controls();
  }
  function clearModels() { modelList = []; $<HTMLSelectElement>('chatgpt-model').replaceChildren(); controls(); }
  async function loadModels(ticket: number, preferred?: string) {
    clearModels(); const current = account();
    if (!current?.connected) { status(current ? 'Sign in again to reconnect this account.' : 'Continue with ChatGPT to connect your plan.'); return; }
    if (!current.planEnabled) { status('Signed in. ChatGPT plan usage is not enabled. Enable it or choose an API key.'); return; }
    status('Loading models for your ChatGPT account…');
    const result = await ChatGPT.models({ accountId: current.id });
    if (ticket !== generation) return;
    modelList = result.models.filter(m => typeof m.id === 'string' && typeof m.name === 'string');
    const select = $<HTMLSelectElement>('chatgpt-model');
    select.replaceChildren(...modelList.map(m => { const option = document.createElement('option'); option.value = m.id; option.textContent = m.name; return option; }));
    if (preferred && modelList.some(m => m.id === preferred)) select.value = preferred;
    status(modelList.length ? 'Using ChatGPT plan. Choose a model that accepts images.' : 'No models are available for this account. Check ChatGPT usage or sign in again.');
  }
  async function run(action: (ticket: number) => Promise<void>) {
    if (busy) return;
    busy = true; const ticket = ++generation; controls();
    try { await action(ticket); }
    catch (error) { if (ticket === generation) status(chatGPTError(error)); }
    finally { if (ticket === generation) { busy = false; authorizing = false; controls(); } }
  }
  async function open(preferred?: ChatGPTSettings) {
    if (!chatGPTMode()) { controls(); return; }
    if (!native) { status('ChatGPT sign-in is available in the installed iPhone or Android app.'); controls(); return; }
    await run(async ticket => {
      clearModels(); status('Checking saved ChatGPT accounts…');
      const next = await ChatGPT.getAccounts(); if (ticket !== generation) return;
      render(next, preferred?.accountId); await loadModels(ticket, preferred?.model);
    });
  }
  async function signIn(existing: boolean) {
    const current = account(); onConnectionChanged();
    await run(async ticket => {
      authorizing = true; controls(); status('Finish signing in with OpenAI, then return to Tandem.');
      const next = await ChatGPT.signIn(existing && current ? { accountId: current.id } : {});
      if (ticket !== generation) return;
      render(next); authorizing = false; await loadModels(ticket);
      if (ticket !== generation) return;
      let welcomed = false; try { welcomed = localStorage.getItem('tandem-chatgpt-welcome') === 'yes'; } catch { /* Optional UI preference. */ }
      if (account()?.connected && account()?.planEnabled && !welcomed) $<HTMLDialogElement>('chatgpt-welcome').showModal();
    });
  }
  function cancel() {
    generation++; busy = false;
    if (authorizing) void ChatGPT.cancelSignIn().catch(() => {});
    authorizing = false; clearModels(); controls();
  }
  $('chatgpt-signin').onclick = () => void signIn(false);
  $('chatgpt-reconnect').onclick = () => void signIn(true);
  $('chatgpt-cancel').onclick = () => { cancel(); status('Sign-in cancelled. Your scans are still here.'); };
  $('chatgpt-accounts').onchange = () => {
    onConnectionChanged(); const current = account();
    if (!current) return;
    void run(async ticket => { clearModels(); const next = await ChatGPT.selectAccount({ accountId: current.id }); if (ticket !== generation) return; render(next, current.id); await loadModels(ticket); });
  };
  $('chatgpt-refresh-models').onclick = () => void run(ticket => loadModels(ticket));
  $('chatgpt-model').onchange = controls;
  $('chatgpt-signout').onclick = () => {
    const current = account(); if (!current) return;
    onConnectionChanged();
    void run(async ticket => {
      clearModels(); const next = await ChatGPT.signOut({ accountId: current.id }); if (ticket !== generation) return;
      render(next, current.id);
      status(next.revoked ? 'Signed out of this ChatGPT account.' : 'Signed out on this phone. Remote sign-out could not be confirmed; disconnect Tandem in ChatGPT settings.');
    });
  };
  for (const id of ['chatgpt-manage', 'scan-manage-usage']) $(id).onclick = () => {
    void manageChatGPTUsage().catch(error => {
      const message = chatGPTError(error);
      if (id === 'scan-manage-usage') $('scan-status').textContent = message;
      else status(message);
    });
  };
  $('chatgpt-welcome-done').onclick = () => {
    try { localStorage.setItem('tandem-chatgpt-welcome', 'yes'); } catch { /* Optional UI preference. */ }
    $<HTMLDialogElement>('chatgpt-welcome').close();
  };
  return {
    open, cancel, controls,
    candidate(): ChatGPTSettings | null {
      const current = account(), model = $<HTMLSelectElement>('chatgpt-model').value;
      if (busy || !current?.connected || !current.planEnabled || !modelList.some(m => m.id === model)) return null;
      return { provider: 'chatgpt', accountId: current.id, accountLabel: current.email || current.label, model };
    },
  };
}
