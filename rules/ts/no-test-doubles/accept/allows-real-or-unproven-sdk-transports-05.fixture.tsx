import Knock from "@knocklabs/node"; new Knock({ fetch: globalThis.fetch.bind(globalThis) });
