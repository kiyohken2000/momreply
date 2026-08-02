import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LangProvider } from "./lang";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root が無い");

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <LangProvider>
      <App />
    </LangProvider>
  </React.StrictMode>,
);
