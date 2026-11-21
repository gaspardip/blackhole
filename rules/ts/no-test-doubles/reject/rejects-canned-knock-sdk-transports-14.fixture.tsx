import Knock from "@knocklabs/node"; new Knock({ fetch: async request => request.method === "GET" ? Response.json({ connections: [] }) : Response.json({ workflow_run_id: "run" }) });
