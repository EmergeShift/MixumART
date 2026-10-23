// Native messaging bridge. The extension is deliberately dumb: it exports only
// what the native host asks for, and the host/Mart show the human consent UI.
const HOST = "dev.mixum.browser";
let port = null;
let reconnectTimer = null;

function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
    port.onMessage.addListener(onNativeMessage);
    port.onDisconnect.addListener(() => {
      port = null;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      reconnectTimer = setTimeout(connect, 3000);
    });
    port.postMessage({ type: "hello", version: "1.0.0" });
  } catch (_) {
    port = null;
    if (reconnectTimer) clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(connect, 3000);
  }
}

function hostOf(url) {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch (_) {
    return "";
  }
}

function domainMatches(host, domain) {
  const d = String(domain || "").toLowerCase().replace(/^\./, "");
  return !!d && (host === d || host.endsWith("." + d));
}

async function exportLease(request) {
  const domains = Array.isArray(request.domains)
    ? request.domains.map((d) => String(d).toLowerCase().replace(/^\./, "")).filter(Boolean)
    : [];
  const urls = Array.isArray(request.urls)
    ? request.urls.map((u) => String(u)).filter(Boolean)
    : [];
  if (domains.length === 0) throw new Error("no approved domains");
  const cookies = [];
  for (const domain of domains) {
    const list = await chrome.cookies.getAll({ domain });
    for (const cookie of list) {
      cookies.push({
        domain: cookie.domain,
        hostOnly: cookie.hostOnly,
        path: cookie.path,
        secure: cookie.secure,
        httpOnly: cookie.httpOnly,
        sameSite: cookie.sameSite,
        session: cookie.session,
        expirationDate: cookie.expirationDate,
        name: cookie.name,
        value: cookie.value,
        storeId: cookie.storeId,
        partitionKey: cookie.partitionKey || null,
      });
    }
  }
  const tabs = await chrome.tabs.query({});
  const selectedTabs = [];
  for (const tab of tabs) {
    const host = hostOf(tab.url || "");
    if (!host || !domains.some((domain) => domainMatches(host, domain))) continue;
    if (urls.length > 0 && !urls.some((url) => tab.url === url || tab.url.startsWith(url))) continue;
    selectedTabs.push({
      url: tab.url,
      title: tab.title || "",
      windowId: tab.windowId,
      index: tab.index,
      groupId: tab.groupId,
      pinned: !!tab.pinned,
    });
  }
  selectedTabs.sort((a, b) => (a.windowId - b.windowId) || (a.index - b.index));
  return { cookies, tabs: selectedTabs, urls, exportedAt: Date.now() };
}

async function onNativeMessage(message) {
  if (!message || typeof message !== "object") return;
  if (message.type === "ping") {
    port?.postMessage({ id: message.id, ok: true, type: "pong", version: "1.0.0" });
    return;
  }
  if (message.type !== "export") return;
  try {
    const lease = await exportLease(message);
    port?.postMessage({ id: message.id, ok: true, lease });
  } catch (error) {
    port?.postMessage({ id: message.id, ok: false, error: String(error?.message || error) });
  }
}

connect();
chrome.runtime.onInstalled.addListener(connect);
chrome.runtime.onStartup.addListener(connect);

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "ping") {
    sendResponse({ ok: !!port, version: "1.0.0" });
  }
  return true;
});
