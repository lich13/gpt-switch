import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import QuickPanel from "./QuickPanel";
import "./style.css";
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {new URLSearchParams(location.search).get("panel") === "quick" ? (
      <QuickPanel />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
