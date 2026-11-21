import Knock from "@knocklabs/node"; new Knock({ fetch: () => Promise.resolve(Response.json({})) });
