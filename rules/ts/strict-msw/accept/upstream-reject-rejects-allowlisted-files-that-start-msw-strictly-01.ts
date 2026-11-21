import { setupWorker } from "msw/browser";
const network = setupWorker(); await network.start({ onUnhandledRequest: "error" });
