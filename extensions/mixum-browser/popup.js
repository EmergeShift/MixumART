const status = document.getElementById("status");
chrome.runtime.sendMessage({ type: "ping" }, (reply) => {
  if (chrome.runtime.lastError || !reply?.ok) {
    status.textContent = "native host not connected — open Chrome once after `mixum browser install`";
    status.className = "bad";
  } else {
    status.textContent = "connected to MixumART native host";
    status.className = "ok";
  }
});
