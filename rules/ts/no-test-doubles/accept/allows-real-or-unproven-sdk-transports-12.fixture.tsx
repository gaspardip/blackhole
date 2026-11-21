import Knock from "@knocklabs/node"; function run(Knock) { new Knock({ fetch: () => Response.json({}) }); }
