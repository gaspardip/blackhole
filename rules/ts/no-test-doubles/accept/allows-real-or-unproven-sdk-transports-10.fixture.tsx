import Knock from "@knocklabs/node"; const old = { fetch: () => Response.json({}) }; new Knock({ ...old, ...unknown });
