import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { applyZoom, readZoom } from "./features/appearance/uiZoom";
import "./styles/base/tokens.css";
import "./styles/base/reset.css";
import "./styles/base/buttons.css";
import "./styles/shell/layout.css";
import "./styles/shell/sidebar.css";
import "./styles/shell/tabs.css";
import "./styles/shell/header.css";
import "./styles/features/chat.css";
import "./styles/components/memory-recall-chip.css";
import "./styles/components/empty-state.css";
import "./styles/components/settings-screen.css";
import "./styles/shared/settings-form.css";
import "./styles/features/models.css";
import "./styles/features/memory-settings.css";
import "./styles/components/tool-call-chip.css";
import "./styles/features/notifications.css";
import "./styles/features/calls.css";
import "./styles/features/import-wizard.css";
import "./styles/features/style-library.css";
import "./styles/features/memory-sky.css";
import "./styles/markdown.css";
import "./styles/syntax.css";

// Before the first render, not after: React's effects run once the tree is
// already on screen, so restoring the zoom there would show one frame at 100%
// and then jump. This is a document-level property, so it needs no tree.
applyZoom(readZoom());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
