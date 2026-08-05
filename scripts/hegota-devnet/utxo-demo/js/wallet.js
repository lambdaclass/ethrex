// wallet.js — wallet connection state, EIP-6963 multi-provider discovery, and helpers.
export const liveInfoPromise = fetch('/api/demos').then((r) => r.json()).catch(() => ({ live: false }));

const state = { account: null, provider: null, providerInfo: null, listeners: [] };
const discovered = new Map(); // rdns -> { info, provider }

// EIP-6963: wallets announce themselves in response to each request, so every
// time we re-request, `discovered` is repopulated with whatever is installed.
window.addEventListener('eip6963:announceProvider', (e) => {
  discovered.set(e.detail.info.rdns, e.detail);
});
window.dispatchEvent(new Event('eip6963:requestProvider'));
if (window.ethereum) {
  discovered.set('injected.default', {
    info: { uuid: 'default', name: 'Browser wallet', icon: '', rdns: 'injected.default' },
    provider: window.ethereum,
  });
}

export function listWallets() {
  return [...discovered.values()];
}

export const walletState = () => state;
export function onWalletChange(fn) { state.listeners.push(fn); fn(state); }
function emit() { state.listeners.forEach((f) => f(state)); }

export const shortAddr = (a) => (a ? a.slice(0, 6) + '…' + a.slice(-4) : '');
export const hasWallet = () => listWallets().length > 0;

export async function connectWallet(detail = null) {
  const choice = detail ?? listWallets()[0];
  if (!choice) throw new Error('no wallet found — install MetaMask, Rainbow, Rabby…');
  state.provider = choice.provider;
  state.providerInfo = choice.info;
  const info = await liveInfoPromise;
  const accounts = await choice.provider.request({ method: 'eth_requestAccounts' });
  state.account = accounts[0];
  if (info.live) await ensureChain(info);
  emit();
  return state.account;
}

async function ensureChain(info) {
  const chainIdHex = '0x' + (3151908).toString(16);
  try {
    await state.provider.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: chainIdHex }] });
  } catch (e) {
    if (e.code === 4902 || /Unrecognized chain/i.test(e.message || '')) {
      await state.provider.request({
        method: 'wallet_addEthereumChain',
        params: [{
          chainId: chainIdHex,
          chainName: 'ethrex EIP-8312 Devnet',
          rpcUrls: [info.rpc],
          nativeCurrency: { name: 'ETH', symbol: 'ETH', decimals: 18 },
        }],
      });
    } else throw e;
  }
}

// Send a vault deposit signed by the visitor: a plain call to UTXO_VAULT with
// the 20-byte recipient as calldata and the value attached — the one
// transaction a normal wallet CAN send in this protocol.
export async function sendWalletDeposit({ to, data, valueWei }) {
  return state.provider.request({
    method: 'eth_sendTransaction',
    params: [{ from: state.account, to, data, value: '0x' + BigInt(valueWei).toString(16) }],
  });
}

export function initWalletListener() {
  // Re-attach accountsChanged whenever a provider is chosen.
  onWalletChange((s) => {
    if (s.provider && !s.provider.__eip8312Listening) {
      s.provider.__eip8312Listening = true;
      s.provider.on('accountsChanged', (accs) => { state.account = accs[0] ?? null; emit(); });
    }
  });
}

// --- wallet picker -------------------------------------------------------------

// Opens a modal listing every discovered wallet; resolves with the connected
// account or null if dismissed.
export function pickWalletAndConnect() {
  // Re-request so freshly installed/unlocked wallets re-announce.
  window.dispatchEvent(new Event('eip6963:requestProvider'));
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'wallet-overlay';
    const modal = document.createElement('div');
    modal.className = 'wallet-modal';
    modal.innerHTML = '<h4>Connect a wallet</h4>';
    const wallets = listWallets();
    if (!wallets.length) {
      modal.innerHTML += '<p class="dim">No injected wallets found. Install MetaMask, Rainbow, Rabby…</p>';
    }
    for (const w of wallets) {
      const item = document.createElement('button');
      item.className = 'wallet-option';
      item.innerHTML = `${w.info.icon ? `<img src="${w.info.icon}" alt="">` : '<span class="wallet-noicon"></span>'}<span>${w.info.name}</span>`;
      item.onclick = async () => {
        item.disabled = true;
        try {
          const acc = await connectWallet(w);
          cleanup();
          resolve(acc);
        } catch (e) {
          item.disabled = false;
          item.insertAdjacentHTML('beforeend', `<span class="err wallet-err">${e.message}</span>`);
        }
      };
      modal.appendChild(item);
    }
    const cancel = document.createElement('button');
    cancel.className = 'btn wallet-cancel';
    cancel.textContent = 'Cancel';
    cancel.onclick = () => { cleanup(); resolve(null); };
    modal.appendChild(cancel);
    overlay.appendChild(modal);
    overlay.onclick = (e) => { if (e.target === overlay) { cleanup(); resolve(null); } };
    document.body.appendChild(overlay);
    function cleanup() { overlay.remove(); }
  });
}
