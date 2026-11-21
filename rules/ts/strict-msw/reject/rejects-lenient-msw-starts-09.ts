import { setupWorker } from "msw/browser";
let network: ReturnType<typeof setupWorker> | undefined;
export async function mount() { network = setupWorker(); await network.start({ onUnhandledRequest: "bypass" }); }
export function unmount() { network = undefined; }
