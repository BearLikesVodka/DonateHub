import { render, updateTime } from "./render.js";

const root = document.getElementById("root");
const params = new URLSearchParams(location.search);

if (params.has("preview")) {
  // Settings page drives the preview with unsaved styles.
  window.addEventListener("message", (e) => {
    if (e.origin === location.origin && e.data?.type === "preview") render(root, e.data.snapshot);
  });
  window.parent.postMessage({ type: "preview-ready" }, location.origin);
} else {
  let delay = 1000;
  const connect = () => {
    const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    ws.onopen = () => { delay = 1000; };
    ws.onmessage = (e) => render(root, JSON.parse(e.data));
    ws.onclose = () => {
      setTimeout(connect, delay);
      delay = Math.min(delay * 2, 10000);
    };
  };
  connect();
}

setInterval(() => updateTime(root), 1000);
