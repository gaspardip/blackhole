import Knock from "@knocklabs/node"; const transport: typeof fetch = async () => Response.json({}); new Knock({ fetch: transport });
