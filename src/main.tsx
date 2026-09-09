import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getStoredTheme } from "./theme";
import "./style.css";

// Applied before the first render (not inside a component effect) so there's
// no flash of the wrong theme on startup.
applyTheme(getStoredTheme());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
