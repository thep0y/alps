import { render } from "solid-js/web";
import App from "./App";
import { applyDocumentLocale, locale } from "./lib/i18n";
import "./index.css";

applyDocumentLocale(locale);

const root = document.getElementById("root");
if (!root) {
  throw new Error("Root element was not found");
}

render(() => <App />, root);
