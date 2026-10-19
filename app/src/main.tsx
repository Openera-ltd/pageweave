import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

// Block the WebView's default context menu (Reload, Inspect, …) everywhere except text fields,
// where the native Cut/Copy/Paste menu is useful. App targets supply their own menus.
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement | null;
  if (t && (t.closest("input, textarea") || t.isContentEditable)) return;
  e.preventDefault();
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
