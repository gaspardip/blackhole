import { setupWorker } from "msw/browser";
let network; network = setupWorker(); await network.start({ quiet: true });
