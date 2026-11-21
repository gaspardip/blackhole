import Knock from "@knocklabs/node"; const old = { fetch: () => Response.json({}) }; const actual = { fetch }; new Knock({ ...old, ...actual });
