import React from "react";
import ReactDOM from "react-dom/client";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import "monaco-editor/esm/vs/editor/edcore.main";
import "monaco-editor/esm/vs/basic-languages/typescript/typescript.contribution";
import "monaco-editor/esm/vs/basic-languages/javascript/javascript.contribution";
import "monaco-editor/esm/vs/basic-languages/rust/rust.contribution";
import "monaco-editor/esm/vs/basic-languages/python/python.contribution";
import "monaco-editor/esm/vs/basic-languages/cpp/cpp.contribution";
import "monaco-editor/esm/vs/basic-languages/csharp/csharp.contribution";
import "monaco-editor/esm/vs/basic-languages/go/go.contribution";
import "monaco-editor/esm/vs/basic-languages/java/java.contribution";
import "monaco-editor/esm/vs/basic-languages/html/html.contribution";
import "monaco-editor/esm/vs/basic-languages/css/css.contribution";
import "monaco-editor/esm/vs/basic-languages/markdown/markdown.contribution";
import "monaco-editor/esm/vs/basic-languages/powershell/powershell.contribution";
import "monaco-editor/esm/vs/basic-languages/bat/bat.contribution";
import "monaco-editor/esm/vs/basic-languages/shell/shell.contribution";
import "monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution";
import "monaco-editor/esm/vs/basic-languages/ini/ini.contribution";
import "monaco-editor/esm/vs/basic-languages/sql/sql.contribution";
import "monaco-editor/esm/vs/basic-languages/xml/xml.contribution";
import "monaco-editor/esm/vs/language/json/monaco.contribution";

import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import { loader } from "@monaco-editor/react";

import "@xterm/xterm/css/xterm.css";
import "./styles/desktop.css";
import App from "./App";

(self as any).MonacoEnvironment = {
  getWorker(_: unknown, label: string) {
    if (label === "json") {
      return new JsonWorker();
    }
    return new EditorWorker();
  },
};

loader.config({ monaco });

// Define signature dark studio Monaco theme
monaco.editor.defineTheme("agent-obsidian", {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "comment", foreground: "64748B", fontStyle: "italic" },
    { token: "keyword", foreground: "C084FC" },
    { token: "string", foreground: "34D399" },
    { token: "number", foreground: "FBBF24" },
    { token: "type", foreground: "38BDF8" },
  ],
  colors: {
    "editor.background": "#0B0F19",
    "editor.foreground": "#E2E8F0",
    "editorLineNumber.foreground": "#475569",
    "editorLineNumber.activeForeground": "#38BDF8",
    "editor.selectionBackground": "#1E3A5F99",
    "editor.lineHighlightBackground": "#131B2E80",
    "editorCursor.foreground": "#38BDF8",
    "diffEditor.insertedTextBackground": "#10B98126",
    "diffEditor.removedTextBackground": "#EF444426",
  },
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
