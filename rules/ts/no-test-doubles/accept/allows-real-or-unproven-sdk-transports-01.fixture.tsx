import Knock from "@knocklabs/node"; new Knock({ fetch: (...args) => Promise.resolve(fetch(...args)) });
