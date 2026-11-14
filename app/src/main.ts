import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { initLang } from "./lib/i18n.svelte";
import { loadUiState } from "./lib/uiState";

// Tabs, theme, language… must be known before the first render.
await loadUiState();
await initLang();

export default mount(App, { target: document.getElementById("app")! });
